import pygame
import sys
import os
import tkinter as tk
from tkinter import filedialog

CELL_SIZE = 32
MARGIN = 8
FONT_W, FONT_H = 8, 8
SCREEN_W, SCREEN_H = CELL_SIZE * FONT_W + MARGIN * 2, CELL_SIZE * FONT_H + MARGIN * 2 + 120

pygame.init()
screen = pygame.display.set_mode((SCREEN_W, SCREEN_H))
pygame.display.set_caption("8x8フォントエディタ")
clock = pygame.time.Clock()

font_data = [[0 for _ in range(FONT_H)] for _ in range(256)]
current_char = 65  # 'A'

def draw_grid(char_code):
    screen.fill((40, 40, 40))
    # Draw grid
    for y in range(FONT_H):
        for x in range(FONT_W):
            bit = (font_data[char_code][y] >> (7 - x)) & 1
            color = (255, 255, 255) if bit else (60, 60, 60)
            pygame.draw.rect(screen, color,
                (MARGIN + x * CELL_SIZE, MARGIN + y * CELL_SIZE, CELL_SIZE - 2, CELL_SIZE - 2))
    # Draw char code
    # 日本語対応フォント指定
    jp_font_name = None
    for name in ["Meiryo", "Yu Gothic", "MS Gothic", "Noto Sans CJK JP"]:
        try:
            font = pygame.font.SysFont(name, 14)
            test = font.render("日本語", True, (0,0,0))
            jp_font_name = name
            break
        except:
            continue
    if jp_font_name:
        font = pygame.font.SysFont(jp_font_name, 14)
        help_font = pygame.font.SysFont(jp_font_name, 14)
    else:
        font = pygame.font.SysFont(None, 14)
        help_font = pygame.font.SysFont(None, 14)
    # 文字表示
    try:
        char_disp = chr(char_code)
    except:
        char_disp = "?"
    # chr(0)（NULL文字）は描画不可なので置換
    if char_code == 0:
        char_disp = " "
    txt = font.render(f"Char: 0x{char_code:02X} ({char_disp})", True, (200, 200, 0))
    screen.blit(txt, (MARGIN, SCREEN_H - 90))
    # ヘルプ
    help_lines = [
        "クリック: ドットON/OFF",
        "←/→: 文字切替  S:保存  L:読込",
        "C:クリア  M:コピー  (8x8 / 2048バイト)"
    ]
    for i, line in enumerate(help_lines):
        t = help_font.render(line, True, (180, 180, 180))
        screen.blit(t, (MARGIN, SCREEN_H - 60 + i * 20))

def save_font():
    root = tk.Tk()
    root.withdraw()
    path = filedialog.asksaveasfilename(
        defaultextension=".f08",
        filetypes=[("Binary font", ("*.bin", "*.f08")), ("All files", "*.*")],
    )
    if not path:
        root.destroy()
        return
    with open(path, "wb") as f:
        f.write(bytes(row for char in font_data for row in char))
    root.destroy()

def load_font():
    root = tk.Tk()
    root.withdraw()
    path = filedialog.askopenfilename(
        filetypes=[("Binary font", ("*.bin", "*.f08")), ("All files", "*.*")],
    )
    if not path:
        root.destroy()
        return
    with open(path, "rb") as f:
        data = f.read()
    expected_size = 256 * FONT_H
    if len(data) != expected_size:
        print(
            f"読込エラー: フォントファイルのサイズが不正です。"
            f"期待値: {expected_size} バイト、実際: {len(data)} バイト"
        )
        root.destroy()
        return
    for char_code in range(256):
        start = char_code * FONT_H
        font_data[char_code] = list(data[start:start + FONT_H])
    root.destroy()

def clear_char(char_code):
    font_data[char_code] = [0 for _ in range(FONT_H)]

def main():
    global current_char
    pygame.key.set_repeat(700, 40)  # キーリピート有効化（初回700ms、以降40ms間隔）
    running = True
    drawing = False
    draw_value = None
    while running:
        draw_grid(current_char)
        pygame.display.flip()
        for event in pygame.event.get():
            if event.type == pygame.QUIT:
                running = False
            elif event.type == pygame.KEYDOWN:
                if event.key == pygame.K_RIGHT:
                    current_char = (current_char + 1) % 256
                elif event.key == pygame.K_LEFT:
                    current_char = (current_char - 1) % 256
                elif event.key == pygame.K_s:
                    save_font()
                elif event.key == pygame.K_l:
                    load_font()
                elif event.key == pygame.K_c:
                    clear_char(current_char)
                elif event.key == pygame.K_m:
                    # コピー機能の追加
                    # ダイアログを表示してコピー元の文字コードを入力
                    root = tk.Tk()
                    root.withdraw()
                    code_str = tk.simpledialog.askstring(
                        "コピー元文字コード",
                        "コピー元の文字コードを16進数で入力 (例: 41):",
                        parent=root,
                    )
                    if code_str:
                        try:
                            src_code = int(code_str, 16)
                            if 0 <= src_code < 256:
                                font_data[current_char] = font_data[src_code][:]
                        except ValueError:
                            pass
                    root.destroy()
            elif event.type == pygame.MOUSEBUTTONDOWN:
                mx, my = event.pos
                gx = (mx - MARGIN) // CELL_SIZE
                gy = (my - MARGIN) // CELL_SIZE
                if 0 <= gx < FONT_W and 0 <= gy < FONT_H:
                    mask = 1 << (7 - gx)
                    if (font_data[current_char][gy] & mask):
                        font_data[current_char][gy] &= ~mask
                        draw_value = 0
                    else:
                        font_data[current_char][gy] |= mask
                        draw_value = 1
                    drawing = True
            elif event.type == pygame.MOUSEBUTTONUP:
                drawing = False
                draw_value = None
            elif event.type == pygame.MOUSEMOTION and drawing:
                mx, my = event.pos
                gx = (mx - MARGIN) // CELL_SIZE
                gy = (my - MARGIN) // CELL_SIZE
                if 0 <= gx < FONT_W and 0 <= gy < FONT_H:
                    mask = 1 << (7 - gx)
                    if draw_value == 1:
                        font_data[current_char][gy] |= mask
                    elif draw_value == 0:
                        font_data[current_char][gy] &= ~mask
        clock.tick(30)
    pygame.quit()
    sys.exit()

if __name__ == "__main__":
    main()