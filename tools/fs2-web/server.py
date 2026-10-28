"""Local FS2 SD-card editor using the Tang BIOS monitor."""
import argparse
import base64
import json
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

class Monitor:
    def __init__(self, name, baud):
        import serial
        self.port = serial.Serial(port=None, baudrate=baud, timeout=3, write_timeout=3)
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
        self.port.write(b'?')
        response = bytearray()
        deadline = time.monotonic() + 4
        while time.monotonic() < deadline:
            b = self.port.read(1)
            response.extend(b)
            if b'boot> ' in response:
                break
        else:
            raise IOError('BIOSモニターが見つかりません。リセット直後に接続してください')
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
    capacity = monitor.connect()
    sb = monitor.read_block(1)
    if sb[:4] != fs2.MAGIC:
        raise ValueError('FS2 superblock が見つかりません')
    count = struct.unpack_from('>I', sb, 8)[0]
    if count < 2 or count > capacity or count > MAX_BLOCKS:
        raise ValueError('FS2サイズが不正か、対応上限の64 MiBを超えています')
    original = bytearray(monitor.read_block(0) + sb)
    for lba in range(2, count):
        original.extend(monitor.read_block(lba))
    return original

def download(monitor, path):
    volume = fs2.Volume(read_volume(monitor))
    inode = volume.resolve(path)
    if inode["type"] != fs2.FILE:
        raise ValueError("指定先はファイルではありません")
    return volume.read(inode)

def update(monitor, path, data):
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
    monitor.write_block(1, dirty)
    for lba in changed:
        if lba != 1:
            monitor.write_block(lba, volume.img[lba*512:(lba+1)*512])
    final = bytearray(volume.img[512:1024])
    struct.pack_into('>H', final, 6, struct.unpack_from('>H', final, 6)[0] | fs2.FLAG_DIRTY)
    if final != dirty:
        monitor.write_block(1, final)
    struct.pack_into('>H', final, 6, struct.unpack_from('>H', final, 6)[0] & ~fs2.FLAG_DIRTY)
    monitor.write_block(1, final)
    return len(changed)

class Handler(BaseHTTPRequestHandler):
    def do_GET(self):
        if self.path != '/':
            self.send_error(404); return
        body = Path(__file__).with_name('index.html').read_bytes()
        self.send_response(200); self.send_header('Content-Type', 'text/html; charset=utf-8')
        self.send_header('Content-Length', str(len(body))); self.end_headers(); self.wfile.write(body)
    def do_POST(self):
        if self.path not in ('/update', '/download') or self.headers.get('Origin') != f'http://127.0.0.1:{self.server.server_port}':
            self.send_error(403); return
        try:
            length = int(self.headers.get('Content-Length', '0'))
            if not 0 < length <= 24 * 1024 * 1024:
                raise ValueError('リクエストサイズが不正です')
            req = json.loads(self.rfile.read(length))
            data = base64.b64decode(req['data'], validate=True) if self.path == '/update' else None
            if not req['path'].startswith('/'):
                raise ValueError('FS2パスは / で始めてください')
            with lock:
                mon = Monitor(req['port'], int(req['baud']))
                try:
                    if self.path == '/update':
                        count = update(mon, req['path'], data)
                        result = {'ok': True, 'message': f'更新完了: {count} セクタ変更'}
                    else:
                        data = download(mon, req['path'])
                        result = {'ok': True, 'data': base64.b64encode(data).decode('ascii')}
                finally: mon.close()
        except Exception as e:
            result = {'ok': False, 'message': str(e)}
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

