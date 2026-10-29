"""Local FS2 SD-card editor using the Tang BIOS monitor."""
import argparse
import base64
import json
import os
import uuid
import struct
import sys
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent / 'fs2'))
import fs2

MAX_BLOCKS = 131072
lock = threading.Lock()
progress = {'message': '', 'percent': None}

class Monitor:
    def __init__(self, name, baud):
        import serial
        self.port = serial.Serial(port=None, baudrate=baud, timeout=0.1, write_timeout=15)
        self.port.dtr = self.port.rts = False
        self.port.port = name
        self.port.open()
    def close(self):
        self.port.close()
    def read_exact(self, n):
        data = bytearray()
        while len(data) < n:
            chunk = self.port.read(n-len(data))
            if not chunk:
                raise IOError('BIOS応答がタイムアウトしました。ボードをリセットしてください')
            data.extend(chunk)
        return bytes(data)
    def reply(self):
        reply = self.read_exact(2)
        if reply[0] != 75 or reply[1]:
            raise IOError(f'BIOS/SD エラー: {reply.hex()}')
    def connect(self):
        self.port.reset_input_buffer()
        response = bytearray()
        deadline = time.monotonic() + 60
        next_probe = 0
        while time.monotonic() < deadline:
            if time.monotonic() >= next_probe:
                self.port.write(b'?')
                next_probe = time.monotonic() + 0.25
            response.extend(self.port.read(128))
            if b'boot> ' in response:
                # Drain replies to probes already queued before issuing I.
                while self.port.read(128):
                    pass
                break
            if len(response) > 4096:
                del response[:-1024]
        else:
            raise IOError('BIOSモニターが見つかりません。接続待機中にボードをリセットしてください')
        # SD initialization can take around 20 seconds on a failed card.
        self.port.timeout = 45
        self.port.write(b'I')
        self.reply()
        return struct.unpack('>I', self.read_exact(4))[0]
    def read_block(self, lba):
        self.port.write(b'D' + struct.pack('>I', lba))
        self.reply()
        data = self.read_exact(512)
        self.reply()
        return data
    def write_block(self, lba, data):
        self.port.write(b'W' + struct.pack('>I', lba))
        self.reply()
        # The monitor polls a small UART FIFO. Bound each burst.
        for pos in range(0, 512, 8):
            self.port.write(data[pos:pos+8])
            time.sleep(0.001)
        self.reply()
        if self.read_block(lba) != data:
            raise IOError(f'セクタ {lba} の読み戻しが一致しません')

def read_volume(monitor):
    progress.update(message='BIOS待機中：必要ならボードをリセットしてください', percent=None)
    capacity = monitor.connect()
    sb = monitor.read_block(1)
    if sb[:4] != fs2.MAGIC:
        raise ValueError('FS2 superblock が見つかりません')
    count = struct.unpack_from('>I', sb, 8)[0]
    if count < 2 or count > capacity or count > MAX_BLOCKS:
        raise ValueError('FS2サイズが不正か、対応上限の64 MiBを超えています')
    original = bytearray(monitor.read_block(0) + sb)
    for lba in range(2, count):
        if lba % 64 == 0:
            progress.update(message=f'FS2読み込み中: {lba}/{count} セクタ', percent=int(90*lba/count))
        original.extend(monitor.read_block(lba))
    if monitor.read_block(1) != sb:
        raise IOError('読み込み中にsuperblockが変化しました')
    progress.update(message='読み込み完了、処理中', percent=90)
    return original

class DirectoryReader:
    """Read only metadata and directory chains needed for a listing.

    This validates the structures it follows, not the rest of the volume.
    """
    def __init__(self, monitor):
        self.monitor = monitor
        capacity = monitor.connect()
        self.cache = {}
        raw = self.block(1)
        self.sb = sb = dict(zip(fs2.SUPER_FIELDS, struct.unpack_from(fs2.SUPER_FMT, raw)))
        total = sb['total_blocks']
        if sb['flags'] == fs2.FLAG_DIRTY:
            raise ValueError('FS2がDIRTYです。必要に応じてレスキューを実行してください')
        if (sb['magic'] != fs2.MAGIC or sb['version'] != fs2.VERSION or
            sb['flags'] or sb['casefold_version'] != fs2.CASEFOLD_VERSION or
            not 2 <= total <= min(capacity, MAX_BLOCKS) or
            not 1 <= sb['inode_count'] <= 0xfffffffe or sb['root_inode'] != 1):
            raise ValueError('FS2 superblock が不正か、DIRTYです')
        expected = fs2.layout(total, sb['boot_blocks'], sb['inode_count'])
        if (any(sb[key] != value for key, value in expected.items()) or
            expected['data_start'] > total or sb['boot_size'] > sb['boot_blocks'] * 512 or
            sb['created_ms'] > 999 or any(raw[fs2.SUPER_USED:])):
            raise ValueError('FS2 superblock のレイアウトが不正です')
    def block(self, number):
        if number not in self.cache:
            self.cache[number] = self.monitor.read_block(number)
            progress.update(message=f'必要なセクタを読み込み中: {len(self.cache)} セクタ', percent=None)
        return self.cache[number]
    def inode(self, number):
        if not 1 <= number <= self.sb['inode_count']:
            raise ValueError('inode 番号が範囲外です')
        offset = (number - 1) * fs2.INODE_SIZE
        raw = self.block(self.sb['inode_start'] + offset // 512)[offset % 512:offset % 512 + fs2.INODE_SIZE]
        inode = dict(zip(fs2.INODE_FIELDS, struct.unpack_from(fs2.INODE_FMT, raw)), number=number)
        if (inode['type'] not in (fs2.FILE, fs2.DIR) or inode['flags'] or inode['reserved'] or
            inode['mode'] & ~fs2.MODE_MASK or inode['uid'] == fs2.INVALID_UID or
            any(raw[0x2a:0x2c]) or any(raw[0x36:0x38]) or any(raw[fs2.INODE_USED:]) or
            any(inode[t+'_ms'] > 999 for t in ('created','modified','changed')) or
            inode['block_count'] != (inode['size'] + fs2.PAYLOAD - 1) // fs2.PAYLOAD or
            (inode['size'] == 0) != (inode['first_block'] == 0)):
            raise ValueError(f'inode {number} が不正です')
        return inode
    def entries(self, inode):
        if inode['type'] != fs2.DIR or inode['size'] % fs2.ENTRY_SIZE:
            raise ValueError('ディレクトリが不正です')
        if inode['block_count'] > self.sb['total_blocks']:
            raise ValueError('ディレクトリのサイズが不正です')
        data = bytearray(); seen = set(); block = inode['first_block']
        for _ in range(inode['block_count']):
            if block in seen or not self.sb['data_start'] <= block < self.sb['total_blocks']:
                raise ValueError('ディレクトリのブロックチェーンが不正です')
            seen.add(block)
            bitmap = self.block(self.sb['bitmap_start'] + block // 4096)
            if not bitmap[(block % 4096) // 8] & (0x80 >> (block % 8)):
                raise ValueError('ディレクトリのブロックが未使用です')
            raw = self.block(block); block = struct.unpack_from('>I', raw)[0]; data.extend(raw[4:])
        if block:
            raise ValueError('ディレクトリのチェーンが長すぎます')
        result = []; names = set()
        for off in range(0, inode['size'], fs2.ENTRY_SIZE):
            raw = data[off:off+fs2.ENTRY_SIZE]; number, length, chars = fs2.ENTRY_HEAD.unpack_from(raw)
            if not number:
                if any(raw): raise ValueError('空エントリが不正です')
                continue
            if length > fs2.NAME_BYTES: raise ValueError('ファイル名が長すぎます')
            name = raw[8:8+length].decode('utf-8')
            if len(name) != chars or fs2.check_name(name) != raw[8:8+length] or any(raw[8+length:]):
                raise ValueError('ディレクトリエントリが不正です')
            key = fs2.fold(name)
            if key in names: raise ValueError('重複するファイル名があります')
            names.add(key); result.append((number, name))
        return result

def list_directory(monitor, path):
    reader = DirectoryReader(monitor)
    inode = reader.inode(1)
    for part in path.split('/'):
        if not part or part == '.': continue
        if part == '..':
            inode = reader.inode(inode['parent']); continue
        matches = [number for number, name in reader.entries(inode) if fs2.fold(name) == fs2.fold(part)]
        if not matches: raise ValueError(f'ディレクトリが見つかりません: {path}')
        inode = reader.inode(matches[0])
    result = []
    for number, name in reader.entries(inode):
        child = reader.inode(number)
        result.append({'name': name, 'directory': child['type'] == fs2.DIR, 'size': child['size']})
    if monitor.read_block(1) != reader.cache[1]:
        raise IOError('読み込み中にsuperblockが変化しました')
    return result

def download(monitor, path):
    reader = DirectoryReader(monitor)
    inode = reader.inode(1)
    parts = [part for part in path.split('/') if part and part != '.']
    for part in parts:
        if part == '..':
            inode = reader.inode(inode['parent']); continue
        matches = [number for number, name in reader.entries(inode) if fs2.fold(name) == fs2.fold(part)]
        if not matches: raise ValueError(f'ファイルが見つかりません: {path}')
        inode = reader.inode(matches[0])
    if inode['type'] != fs2.FILE:
        raise ValueError('指定先はファイルではありません')
    if inode['block_count'] > reader.sb['total_blocks']:
        raise ValueError('ファイルのサイズが不正です')
    data = bytearray(); seen = set(); block = inode['first_block']
    for _ in range(inode['block_count']):
        if block in seen or not reader.sb['data_start'] <= block < reader.sb['total_blocks']:
            raise ValueError('ファイルのブロックチェーンが不正です')
        seen.add(block)
        bitmap = reader.block(reader.sb['bitmap_start'] + block // 4096)
        if not bitmap[(block % 4096) // 8] & (0x80 >> (block % 8)):
            raise ValueError('ファイルのブロックが未使用です')
        raw = reader.block(block); block = struct.unpack_from('>I', raw)[0]; data.extend(raw[4:])
    if block: raise ValueError('ファイルのチェーンが長すぎます')
    if monitor.read_block(1) != reader.cache[1]:
        raise IOError('読み込み中にsuperblockが変化しました')
    return bytes(data[:inode['size']])

def validate_upload_path(path):
    if not isinstance(path, str) or not path.startswith('/') or path.endswith('/'):
        raise ValueError('アップロード先には /ファイル名 を含む絶対パスを指定してください')
    fs2.check_name(path.rsplit('/', 1)[-1])

def update(monitor, path, data):
    validate_upload_path(path)
    original = read_volume(monitor)
    count = len(original) // 512
    sb = original[512:1024]
    volume = fs2.Volume(bytearray(original))
    volume.put(path, data)
    problems = fs2.check_image(volume.img)
    if problems:
        raise ValueError(f'更新後のFS2検証失敗: {problems[0]}')
    changed = [i for i in range(count) if original[i*512:(i+1)*512] != volume.img[i*512:(i+1)*512]]
    if not changed:
        return 0
    dirty = bytearray(sb)
    struct.pack_into('>H', dirty, 6, struct.unpack_from('>H', dirty, 6)[0] | fs2.FLAG_DIRTY)
    progress.update(message='DIRTYを設定中', percent=90)
    monitor.write_block(1, dirty)
    for index, lba in enumerate(changed):
        progress.update(message=f'更新中: {index+1}/{len(changed)} セクタ', percent=90+int(9*index/len(changed)))
        if lba != 1:
            monitor.write_block(lba, volume.img[lba*512:(lba+1)*512])
    final = bytearray(volume.img[512:1024])
    struct.pack_into('>H', final, 6, struct.unpack_from('>H', final, 6)[0] | fs2.FLAG_DIRTY)
    if final != dirty:
        monitor.write_block(1, final)
    struct.pack_into('>H', final, 6, struct.unpack_from('>H', final, 6)[0] & ~fs2.FLAG_DIRTY)
    monitor.write_block(1, final)
    return len(changed)

def validate_backup(data):
    if len(data) < 1024 or len(data) % fs2.BLOCK or len(data) > MAX_BLOCKS * fs2.BLOCK:
        raise ValueError('バックアップは512バイト単位、最大64 MiBのFS2イメージである必要があります')
    total = struct.unpack_from('>I', data, fs2.BLOCK + 8)[0]
    if total != len(data) // fs2.BLOCK:
        raise ValueError('イメージ長とFS2のボリュームサイズが一致しません')
    problems = fs2.check_image(data, allow_dirty=True)
    if problems:
        raise ValueError(f'バックアップのFS2検査に失敗しました: {problems[0]}')
    return total


def restore(monitor, data):
    total = validate_backup(data)
    progress.update(message='BIOS待機中：必要ならボードをリセットしてください', percent=None)
    capacity = monitor.connect()
    if total > capacity:
        raise ValueError('バックアップがSDカード容量を超えています。書き込みは行っていません')
    final = data[512:1024]
    dirty = bytearray(final)
    struct.pack_into('>H', dirty, 6, struct.unpack_from('>H', dirty, 6)[0] | fs2.FLAG_DIRTY)
    progress.update(message='復元開始：DIRTYを設定中', percent=0)
    try:
        monitor.write_block(1, dirty)
        for lba in range(total):
            if lba == 1:
                continue
            progress.update(message=f'復元・読み戻し中: {lba + 1}/{total} セクタ',
                            percent=int(99 * (lba + 1) / total))
            monitor.write_block(lba, data[lba*512:(lba+1)*512])
        if final != dirty:
            progress.update(message='復元完了：DIRTYを解除中', percent=99)
            monitor.write_block(1, final)
    except Exception as exc:
        raise IOError('復元が中断しました。媒体は不完全またはDIRTYの可能性があります。'
                      'バックアップから再度復元してください: ' + str(exc)) from exc
    return total, bool(struct.unpack_from('>H', final, 6)[0] & fs2.FLAG_DIRTY)

def prepare_rescue(image):
    """Repair allocation leaks only; refuse ambiguous damage without writing."""
    if len(image) < 1024 or len(image) % 512:
        raise ValueError('イメージサイズが不正です')
    sb = dict(zip(fs2.SUPER_FIELDS, struct.unpack_from(fs2.SUPER_FMT, image, 512)))
    count = sb['total_blocks']
    if (image[510:512] != fs2.BOOT_SIG or sb['magic'] != fs2.MAGIC or
        sb['version'] != fs2.VERSION or sb['flags'] != fs2.FLAG_DIRTY or
        sb['casefold_version'] != fs2.CASEFOLD_VERSION or count != len(image)//512 or
        not 2 <= count <= MAX_BLOCKS or not 1 <= sb['inode_count'] <= 0xfffffffe or
        sb['root_inode'] != fs2.ROOT_INODE):
        raise ValueError('DIRTYなFS2のsuperblockが不正または未対応です')
    expected = fs2.layout(count, sb['boot_blocks'], sb['inode_count'])
    if (any(sb[k] != v for k, v in expected.items()) or expected['data_start'] > count or
        sb['boot_size'] > sb['boot_blocks'] * 512 or sb['created_ms'] > 999):
        raise ValueError('FS2のレイアウトが不正です')
    repaired = bytearray(image)
    vol = fs2.Volume(repaired, check=False)
    chains = {}; owners = set()
    for number in range(1, sb['inode_count'] + 1):
        inode = vol.read_inode(number)
        if inode['type'] == fs2.FREE:
            continue
        if (inode['type'] not in (fs2.FILE, fs2.DIR) or
            inode['block_count'] != (inode['size'] + fs2.PAYLOAD - 1) // fs2.PAYLOAD or
            inode['block_count'] > count or
            (inode['size'] == 0) != (inode['first_block'] == 0)):
            raise ValueError(f'inode {number} のサイズまたは種類が不正です')
        block = inode['first_block']; chain = []
        for _ in range(inode['block_count']):
            if not sb['data_start'] <= block < count or block in owners:
                raise ValueError(f'inode {number} のブロック範囲外・循環・共有を検出しました')
            owners.add(block); chain.append(block); block = vol.next_block(block)
        if block:
            raise ValueError(f'inode {number} のチェーンが長すぎます')
        chains[number] = chain
    start = sb['bitmap_start'] * 512
    repaired[start:start + sb['bitmap_blocks']*512] = bytes(sb['bitmap_blocks']*512)
    for block in range(sb['data_start']): vol.set_bit(block, True)
    for block in owners: vol.set_bit(block, True)
    problems = fs2.check_image(repaired, allow_dirty=True)
    orphan = []
    for code, message in problems:
        words = message.split()
        if len(words) == 8 and words[0] == 'inode' and words[2:] == ['is','not','reachable','from','the','root']:
            orphan.append(int(words[1]))
        else:
            raise ValueError(f'自動修復できないFS2破損: {code}: {message}')
    for number in orphan:
        off = vol.inode_offset(number)
        repaired[off:off+fs2.INODE_SIZE] = bytes(fs2.INODE_SIZE)
        for block in chains[number]:
            vol.set_bit(block, False)
            repaired[block*512:(block+1)*512] = bytes(512)
    problems = fs2.check_image(repaired, allow_dirty=True)
    if problems:
        raise ValueError(f'修復後の検査失敗: {problems[0]}')
    return repaired, orphan


def rescue(monitor, backup_dir):
    original = read_volume(monitor)
    if not struct.unpack_from('>H', original, 518)[0] & fs2.FLAG_DIRTY:
        raise ValueError('DIRTYではないためレスキューは実行しません')
    backup_dir = Path(backup_dir)
    backup_dir.mkdir(parents=True, exist_ok=True)
    backup = backup_dir / ('fs2-rescue-' + uuid.uuid4().hex + '.bin')
    with backup.open('xb') as output:
        output.write(original); output.flush(); os.fsync(output.fileno())
    try:
        repaired, orphan = prepare_rescue(original)
        changed = [i for i in range(len(original)//512) if i != 1 and
                   original[i*512:(i+1)*512] != repaired[i*512:(i+1)*512]]
        if monitor.read_block(1) != original[512:1024]:
            raise ValueError('読み込み後にsuperblockが変化しました')
        for index, lba in enumerate(changed):
            progress.update(message=f'レスキュー書き込み中: {index+1}/{len(changed)} セクタ',
                            percent=90 + 9*index//max(1,len(changed)))
            monitor.write_block(lba, repaired[lba*512:(lba+1)*512])
        final = bytearray(repaired[512:1024])
        struct.pack_into('>H', final, 6, struct.unpack_from('>H', final, 6)[0] & ~fs2.FLAG_DIRTY)
        monitor.write_block(1, final)
    except Exception as exc:
        raise IOError(f'レスキュー失敗。元イメージ: {backup}。媒体はDIRTYまたは不完全の可能性があります: {exc}') from exc
    return str(backup), len(orphan), len(changed)

class Handler(BaseHTTPRequestHandler):
    def log_message(self, format, *args):
        if self.path != '/progress':
            super().log_message(format, *args)

    def do_GET(self):
        if self.path == '/progress':
            body = json.dumps(progress, ensure_ascii=False).encode()
            content_type = 'application/json; charset=utf-8'
        elif self.path == '/':
            body = Path(__file__).with_name('index.html').read_bytes()
            content_type = 'text/html; charset=utf-8'
        else:
            self.send_error(404); return
        self.send_response(200); self.send_header('Content-Type', content_type)
        self.send_header('Content-Length', str(len(body))); self.end_headers(); self.wfile.write(body)
    def do_POST(self):
        if self.path not in ('/update', '/download', '/list', '/backup', '/restore', '/rescue') or self.headers.get('Origin') != f'http://127.0.0.1:{self.server.server_port}':
            self.send_error(403); return
        try:
            length = int(self.headers.get('Content-Length', '0'))
            if not 0 < length <= 90 * 1024 * 1024:
                raise ValueError('リクエストサイズが不正です')
            req = json.loads(self.rfile.read(length))
            data = base64.b64decode(req['data'], validate=True) if self.path in ('/update', '/restore') else None
            if self.path == '/update':
                validate_upload_path(req.get('path'))
            if self.path == '/restore':
                validate_backup(data)  # reject bad images before opening the port
            if not req.get('path', '/').startswith('/'):
                raise ValueError('FS2パスは / で始めてください')
            with lock:
                mon = Monitor(req['port'], int(req['baud']))
                try:
                    if self.path == '/update':
                        count = update(mon, req['path'], data)
                        result = {'ok': True, 'message': f'更新完了: {count} セクタ変更'}
                    elif self.path == '/restore':
                        count, dirty = restore(mon, data)
                        result = {'ok': True, 'message': f'復元完了: {count} セクタ。' + ('元のDIRTYを維持しています。' if dirty else 'DIRTYは解除されています。')}
                    elif self.path == '/rescue':
                        saved, orphan, count = rescue(mon, Path(__file__).resolve().parents[2] / 'target' / 'fs2-web-rescue')
                        result = {'ok': True, 'message': f'レスキュー完了。解放inode: {orphan}、変更セクタ: {count}。元イメージ: {saved}'}
                    elif self.path == '/backup':
                        data = read_volume(mon)
                        result = {'ok': True, 'data': base64.b64encode(data).decode('ascii')}
                    elif self.path == '/list':
                        result = {'ok': True, 'entries': list_directory(mon, req['path'])}
                    else:
                        data = download(mon, req['path'])
                        result = {'ok': True, 'data': base64.b64encode(data).decode('ascii')}
                finally: mon.close()
        except Exception as e:
            result = {'ok': False, 'message': str(e)}
        progress.update(message='', percent=None)
        body = json.dumps(result, ensure_ascii=False).encode()
        self.send_response(200); self.send_header('Content-Type', 'application/json; charset=utf-8')
        self.send_header('Content-Length', str(len(body))); self.end_headers(); self.wfile.write(body)

def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--port', type=int, default=8765, help='local HTTP port')
    args = parser.parse_args()
    server = ThreadingHTTPServer(('127.0.0.1', args.port), Handler)
    print(f'http://127.0.0.1:{server.server_port}/', flush=True)
    server.serve_forever()

if __name__ == '__main__': main()

