#!/usr/bin/env python3
"""Gera o ícone pixel-art do Maestro: equalizer de atividade em fundo escuro.

Desenhado num grid lógico de 32x32 e escalado ×32 (nearest) para 1024x1024,
mantendo as bordas nítidas (pixel art de verdade).
"""

from PIL import Image, ImageDraw

GRID = 32
SCALE = 32  # 32*32 = 1024px

# Paleta (mesma da UI)
BG = (14, 16, 20, 255)        # near-black do painel
BORDER = (40, 44, 54, 255)    # contorno sutil
VIOLET_HI = (157, 140, 255, 255)
VIOLET = (124, 108, 240, 255)
GREEN = (88, 201, 138, 255)
AMBER = (230, 180, 80, 255)

img = Image.new("RGBA", (GRID, GRID), (0, 0, 0, 0))
d = ImageDraw.Draw(img)

# Fundo arredondado (cantos transparentes), sem antialias = pixel art limpo
d.rounded_rectangle([0, 0, GRID - 1, GRID - 1], radius=7, fill=BG, outline=BORDER, width=1)

# Barras do equalizer: (x, largura, topo, cor). Baseline em y=25.
BASE = 25
bars = [
    (7, 3, 13, VIOLET_HI),
    (12, 3, 7, VIOLET),
    (17, 3, 16, GREEN),
    (22, 3, 10, VIOLET),
]
for x, w, top, color in bars:
    d.rectangle([x, top, x + w - 1, BASE], fill=color)
    # topo levemente arredondado: apaga os 2 pixels de canto do topo
    img.putpixel((x, top), (0, 0, 0, 0))
    img.putpixel((x + w - 1, top), (0, 0, 0, 0))
    # repinta o fundo nesses cantos para não furar o painel
    for cx in (x, x + w - 1):
        if 1 <= cx <= GRID - 2 and 1 <= top <= GRID - 2:
            img.putpixel((cx, top), BG)

# "batuta": um ponto de acento acima da barra mais alta
img.putpixel((13, 4), VIOLET_HI)

# Escala mantendo os pixels duros
big = img.resize((GRID * SCALE, GRID * SCALE), Image.NEAREST)
big.save("icon.png")
print(f"icon.png gerado: {big.size[0]}x{big.size[1]}")
