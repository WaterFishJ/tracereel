"""把三张完整概念图的右侧控件区拼成一张对比图。"""
from pathlib import Path
from PIL import Image, ImageDraw, ImageFont

HERE = Path(__file__).resolve().parent
RENDERS = HERE / "renders"
ITEMS = [
    ("01-professional-dark.png", "方案一 · 专业剪辑暗色"),
    ("02-fluent-light.png", "方案二 · Fluent 浅色"),
    ("03-linear-modern.png", "方案三 · 现代极简"),
]
MARGIN = 24
GAP = 20
HEADER = 78
CROP = (968, 64, 1440, 960)

font_path = Path(r"C:\Windows\Fonts\msyh.ttc")
font = ImageFont.truetype(str(font_path), 24)
small = ImageFont.truetype(str(font_path), 15)
width = MARGIN * 2 + 472 * 3 + GAP * 2
height = HEADER + 896 + MARGIN
sheet = Image.new("RGB", (width, height), "#EEF0F3")
draw = ImageDraw.Draw(sheet)
for index, (filename, title) in enumerate(ITEMS):
    x = MARGIN + index * (472 + GAP)
    draw.text((x, 15), title, font=font, fill="#202124")
    draw.text((x, 48), "相同内容与控件，仅改变视觉语言", font=small, fill="#69707A")
    image = Image.open(RENDERS / filename).convert("RGB").crop(CROP)
    sheet.paste(image, (x, HEADER))
sheet.save(RENDERS / "04-control-comparison.png")
print(RENDERS / "04-control-comparison.png")
