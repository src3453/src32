#!/usr/bin/env python3
"""Convert a Wavefront OBJ (and MTL diffuse textures) into a self-contained sol program.

The generated program embeds the VPU command stream as ``!data`` words. Float
attributes are stored as big-endian IEEE-754 binary32 bit patterns, just like
the VPU command FIFO expects. Pillow is required to decode and resize images.
"""

from __future__ import annotations

import argparse
import math
import struct
import sys
from dataclasses import dataclass, field
from pathlib import Path
from typing import Iterable


VPU_FIFO_DATA = 0x8003002C
VPU_STATUS = 0x80030004
VDP_ENABLE = 0x80000000
VDP_MODE = 0x80000001
VDP_BORDER = 0x80000003
TEXTURE_OPCODE = 0x17
TEXTURED_LIT_TRIANGLE_OPCODE = 0x19
WHITE_TEXTURE = (255, 255, 255, 255)


class ObjConvertError(ValueError):
    pass


@dataclass
class Material:
    name: str
    diffuse: tuple[float, float, float] = (1.0, 1.0, 1.0)
    texture_path: Path | None = None
    texture_id: int = 0


@dataclass
class Face:
    corners: list[tuple[int, int | None, int | None]]
    material: str


@dataclass
class ObjModel:
    positions: list[tuple[float, float, float]] = field(default_factory=list)
    texcoords: list[tuple[float, float]] = field(default_factory=list)
    normals: list[tuple[float, float, float]] = field(default_factory=list)
    faces: list[Face] = field(default_factory=list)
    mtllibs: list[Path] = field(default_factory=list)


def f32_word(value: float) -> int:
    if not math.isfinite(value):
        raise ObjConvertError(f"non-finite vertex/UV value: {value}")
    try:
        return struct.unpack(">I", struct.pack(">f", value))[0]
    except (OverflowError, struct.error) as exc:
        raise ObjConvertError(f"value is outside binary32 range: {value}") from exc


def obj_index(token: str, count: int, label: str, line_no: int) -> int:
    try:
        index = int(token)
    except ValueError as exc:
        raise ObjConvertError(f"OBJ line {line_no}: invalid {label} index {token!r}") from exc
    if index == 0:
        raise ObjConvertError(f"OBJ line {line_no}: {label} index cannot be zero")
    resolved = index - 1 if index > 0 else count + index
    if not 0 <= resolved < count:
        raise ObjConvertError(f"OBJ line {line_no}: {label} index {index} is out of range")
    return resolved


def parse_obj(path: Path) -> ObjModel:
    model = ObjModel()
    active_material = "__default__"
    try:
        lines = path.read_text(encoding="utf-8-sig").splitlines()
    except OSError as exc:
        raise ObjConvertError(f"cannot read OBJ {path}: {exc}") from exc

    for line_no, raw in enumerate(lines, 1):
        line = raw.partition("#")[0].strip()
        if not line:
            continue
        fields = line.split()
        op, args = fields[0], fields[1:]
        try:
            if op == "v":
                if len(args) < 3:
                    raise ObjConvertError(f"OBJ line {line_no}: vertex needs x y z")
                model.positions.append(tuple(float(v) for v in args[:3]))
            elif op == "vt":
                if len(args) < 2:
                    raise ObjConvertError(f"OBJ line {line_no}: texture coordinate needs u v")
                model.texcoords.append((float(args[0]), float(args[1])))
            elif op == "vn":
                if len(args) < 3:
                    raise ObjConvertError(f"OBJ line {line_no}: normal needs x y z")
                model.normals.append(tuple(float(v) for v in args[:3]))
            elif op == "mtllib":
                model.mtllibs.extend((path.parent / name).resolve() for name in args)
            elif op == "usemtl":
                active_material = " ".join(args) if args else "__default__"
            elif op == "f":
                if len(args) < 3:
                    raise ObjConvertError(f"OBJ line {line_no}: face needs at least three vertices")
                corners: list[tuple[int, int | None, int | None]] = []
                for item in args:
                    parts = item.split("/")
                    vi = obj_index(parts[0], len(model.positions), "vertex", line_no)
                    ti = None
                    if len(parts) > 1 and parts[1]:
                        ti = obj_index(parts[1], len(model.texcoords), "texture", line_no)
                    ni = None
                    if len(parts) > 2 and parts[2]:
                        ni = obj_index(parts[2], len(model.normals), "normal", line_no)
                    corners.append((vi, ti, ni))
                model.faces.append(Face(corners, active_material))
        except (ValueError, OverflowError) as exc:
            if isinstance(exc, ObjConvertError):
                raise
            raise ObjConvertError(f"OBJ line {line_no}: invalid numeric value") from exc

    if not model.positions:
        raise ObjConvertError("OBJ contains no vertices")
    if not model.faces:
        raise ObjConvertError("OBJ contains no faces")
    add_missing_vertex_normals(model)
    return model


def add_missing_vertex_normals(model: ObjModel) -> None:
    """Generate area-weighted smooth normals for OBJ corners without ``vn``."""
    accumulated = [[0.0, 0.0, 0.0] for _ in model.positions]
    for face in model.faces:
        first = face.corners[0][0]
        a = model.positions[first]
        for i in range(1, len(face.corners) - 1):
            b = model.positions[face.corners[i][0]]
            c = model.positions[face.corners[i + 1][0]]
            ab = tuple(b[k] - a[k] for k in range(3))
            ac = tuple(c[k] - a[k] for k in range(3))
            normal = (ab[1] * ac[2] - ab[2] * ac[1],
                      ab[2] * ac[0] - ab[0] * ac[2],
                      ab[0] * ac[1] - ab[1] * ac[0])
            for pos_index, _, _ in (face.corners[0], face.corners[i], face.corners[i + 1]):
                for axis in range(3):
                    accumulated[pos_index][axis] += normal[axis]

    generated_indices: dict[int, int] = {}
    for position_index, normal in enumerate(accumulated):
        length = math.sqrt(sum(value * value for value in normal))
        normalized = tuple(value / length for value in normal) if length > 1e-12 else (0.0, 0.0, 1.0)
        generated_indices[position_index] = len(model.normals)
        model.normals.append(normalized)
    for face in model.faces:
        face.corners = [
            (position, uv, generated_indices[position] if normal is None else normal)
            for position, uv, normal in face.corners
        ]


def parse_mtl(paths: Iterable[Path]) -> dict[str, Material]:
    materials: dict[str, Material] = {}
    for path in paths:
        try:
            lines = path.read_text(encoding="utf-8-sig").splitlines()
        except OSError as exc:
            raise ObjConvertError(f"cannot read MTL {path}: {exc}") from exc
        current: Material | None = None
        for line_no, raw in enumerate(lines, 1):
            line = raw.partition("#")[0].strip()
            if not line:
                continue
            fields = line.split()
            op, args = fields[0], fields[1:]
            if op == "newmtl":
                if not args:
                    raise ObjConvertError(f"MTL {path}:{line_no}: newmtl needs a name")
                current = Material(" ".join(args))
                materials[current.name] = current
            elif current is not None and op == "Kd":
                if len(args) < 3:
                    raise ObjConvertError(f"MTL {path}:{line_no}: Kd needs three components")
                try:
                    current.diffuse = tuple(float(v) for v in args[:3])
                except ValueError as exc:
                    raise ObjConvertError(f"MTL {path}:{line_no}: invalid Kd") from exc
            elif current is not None and op == "map_Kd":
                # MTL options are varied in the wild. Resolve the final token as
                # the filename, which also supports the common -s/-o options.
                if not args:
                    raise ObjConvertError(f"MTL {path}:{line_no}: map_Kd needs a filename")
                current.texture_path = (path.parent / args[-1]).resolve()
    return materials


def rgba8(rgb: tuple[float, float, float]) -> int:
    channels = [max(0, min(255, round(value * 255))) for value in rgb]
    return (channels[0] << 24) | (channels[1] << 16) | (channels[2] << 8) | 0xFF


_BAYER_4X4 = (
    (0, 8, 2, 10),
    (12, 4, 14, 6),
    (3, 11, 1, 9),
    (15, 7, 13, 5),
)


def quantize_rgb4(channel: int, x: int, y: int) -> int:
    """Ordered-dither one 8-bit color channel to 4 bits using a 4x4 Bayer matrix."""
    channel = max(0, min(255, int(channel)))
    level, remainder = divmod(channel, 17)
    threshold = _BAYER_4X4[y & 3][x & 3]
    # Compare against centered thresholds so each Bayer cell covers one of 16 bins.
    if level < 15 and remainder * 32 > 17 * (threshold * 2 + 1):
        level += 1
    return level


def rgba4444(pixel: tuple[int, int, int, int], x: int = 0, y: int = 0) -> int:
    r, g, b, a = (max(0, min(255, int(channel))) for channel in pixel)
    return (
        (quantize_rgb4(r, x, y) << 12)
        | (quantize_rgb4(g, x, y) << 8)
        | (quantize_rgb4(b, x, y) << 4)
        | (a >> 4)
    )


def load_texture(path: Path | None) -> tuple[int, int, list[int]]:
    if path is None:
        return 1, 1, [rgba4444(WHITE_TEXTURE)]
    try:
        from PIL import Image
    except ImportError as exc:
        raise ObjConvertError("texture conversion needs Pillow; install it with `python -m pip install Pillow`") from exc
    try:
        with Image.open(path) as source:
            image = source.convert("RGBA")
            if image.width < 1 or image.height < 1:
                raise ObjConvertError(f"texture has an invalid size: {path}")
            if image.width > 1024 or image.height > 1024:
                scale = min(1024 / image.width, 1024 / image.height)
                size = (max(1, round(image.width * scale)), max(1, round(image.height * scale)))
                image = image.resize(size, Image.Resampling.LANCZOS)
            width, height = image.size
            pixels = [
                rgba4444(pixel, index % width, index // width)
                for index, pixel in enumerate(image.getdata())
            ]
    except ObjConvertError:
        raise
    except Exception as exc:
        raise ObjConvertError(f"cannot decode texture {path}: {exc}") from exc
    return width, height, pixels


def packed_pixels(pixels: list[int]) -> list[int]:
    return [(pixels[i] << 16) | (pixels[i + 1] if i + 1 < len(pixels) else 0)
            for i in range(0, len(pixels), 2)]


def texture_commands(model: ObjModel, materials: dict[str, Material]) -> tuple[list[int], dict[str, Material], int]:
    used_materials = {face.material for face in model.faces}
    # Ensure every OBJ material maps to a defined texture, falling back to white.
    resolved: dict[str, Material] = {}
    textures: list[tuple[int, int, list[int]]] = []
    paths_to_id: dict[Path | None, int] = {}
    for material_name in sorted(used_materials):
        mat = materials.get(material_name, Material(material_name))
        key = mat.texture_path
        if key not in paths_to_id:
            tex_id = len(textures) + 1
            paths_to_id[key] = tex_id
            textures.append(load_texture(key))
        mat.texture_id = paths_to_id[key]
        resolved[material_name] = mat
    words: list[int] = []
    for tex_id, (width, height, pixels) in enumerate(textures, 1):
        data = packed_pixels(pixels)
        words.extend([((TEXTURE_OPCODE << 24) | (3 + len(data))), tex_id, width, height, *data])
    return words, resolved, len(textures)


def model_commands(model: ObjModel, materials: dict[str, Material]) -> tuple[list[int], int, int]:
    # Center the object and fit its largest axis into [-1, 1]. The generated
    # view/projection matrices then frame ordinary OBJ assets consistently.
    mins = [min(position[axis] for position in model.positions) for axis in range(3)]
    maxs = [max(position[axis] for position in model.positions) for axis in range(3)]
    center = [(mins[axis] + maxs[axis]) * 0.5 for axis in range(3)]
    extent = max(maxs[axis] - mins[axis] for axis in range(3))
    if extent <= 0.0:
        raise ObjConvertError("OBJ vertices have no spatial extent")
    scale = 2.0 / extent

    # Three row-major binary32 matrices: identity model, view translated to
    # z=3.5, and a perspective projection with an infinite far plane.
    identity = [1.0, 0.0, 0.0, 0.0,
                0.0, 1.0, 0.0, 0.0,
                0.0, 0.0, 1.0, 0.0,
                0.0, 0.0, 0.0, 1.0]
    view = [1.0, 0.0, 0.0, 0.0,
            0.0, 1.0, 0.0, 0.0,
            0.0, 0.0, 1.0, 3.5,
            0.0, 0.0, 0.0, 1.0]
    projection = [1.5, 0.0, 0.0, 0.0,
                  0.0, 2.0, 0.0, 0.0,
                  0.0, 0.0, 1.0, -1.0,
                  0.0, 0.0, 1.0, 0.0]
    words: list[int] = []
    for matrix_id, matrix in enumerate((identity, view, projection)):
        words.extend([(0x02 << 24) | 17, matrix_id, *(f32_word(v) for v in matrix)])
    # One point light in view space: index/enabled, ambient, diffuse, specular,
    # emission, and position (all binary32 except the first two words).
    words.extend([
        (0x15 << 24) | 17, 0, 1,
        *(f32_word(v) for v in (0.25, 0.25, 0.25)),
        *(f32_word(v) for v in (0.80, 0.80, 0.80)),
        *(f32_word(v) for v in (0.15, 0.15, 0.15)),
        *(f32_word(v) for v in (0.0, 0.0, 0.0)),
        # The camera is at view-space z=0 and looks toward +Z, so the visible
        # outside surface of a normally wound mesh faces toward -Z. Keep the
        # light on that camera side, well outside the model at z=[2.5, 4.5].
        *(f32_word(v) for v in (0.0, 0.0, -100.0)),
    ])
    # Enable fixed-function lighting (state ID 1).
    words.extend([(0x01 << 24) | 2, 1, 1])
    # Clear color and depth before drawing this one-shot model frame.
    words.extend([(0x04 << 24) | 3, 3, 0x101018FF, f32_word(1.0)])
    setup_words = len(words)
    triangles = 0
    for face in model.faces:
        material = materials[face.material]
        # Fan triangulation preserves face winding for convex polygons.
        for i in range(1, len(face.corners) - 1):
            corners = (face.corners[0], face.corners[i], face.corners[i + 1])
            # Per-triangle data record: texture ID plus 27 vertex words
            # (position, normal, UV, RGBA8). The selected shading mode is
            # inserted by the sol runtime loop.
            words.append(material.texture_id)
            color = rgba8(material.diffuse)
            for position_index, texcoord_index, normal_index in corners:
                raw_position = model.positions[position_index]
                x, y, z = ((raw_position[axis] - center[axis]) * scale for axis in range(3))
                nx, ny, nz = model.normals[normal_index]
                u, v = model.texcoords[texcoord_index] if texcoord_index is not None else (0.0, 0.0)
                words.extend([
                    f32_word(x), f32_word(y), f32_word(z),
                    f32_word(nx), f32_word(ny), f32_word(nz),
                    f32_word(u), f32_word(1.0 - v), color,
                ])
            triangles += 1
    return words, triangles, setup_words


def data_block(words: list[int]) -> str:
    if not words:
        return "!data\n!end"
    lines = ["!data"]
    for start in range(0, len(words), 8):
        lines.append("    " + " ".join(f"0x{word:08X}" for word in words[start:start + 8]))
    lines.append("!end")
    return "\n".join(lines)


def generate_sol(model: ObjModel, texture_words: list[int], draw_words: list[int], setup_words: int, triangles: int, data_base: int, rotate: bool = False) -> str:
    if data_base & 3:
        raise ObjConvertError("data base address must be 4-byte aligned")
    model_base = data_base + 4 * len(texture_words)
    model_data_base = model_base + 4 * setup_words
    model_stream = (
        [(0x1A << 24) | (2 + 28 * triangles), 0, triangles, *draw_words[setup_words:]]
        if rotate else draw_words[setup_words:]
    )
    model_words = [*draw_words[:setup_words], *model_stream]
    dma_total_bytes = 4 * (len(texture_words) + len(model_words))
    if model_base + 4 * len(model_words) > 0x00100000:
        raise ObjConvertError("embedded blobs exceed the sol read-only data region (0x20000..0xFFFFF)")
    rotation_helpers = '''
fn irq0 () :
    1 IRQC_PENDING sth
    retn
;

fn q10_to_f32 (value) :
    local sign 0
    local magnitude
    local scale -10
    value >magnitude
    value 0 lt if
        0x80000000 >sign
        magnitude neg >magnitude
    end
    magnitude 0 eq if
        0 ret
    end
    while
        magnitude 1024 ge if
            magnitude 1 shr >magnitude
            scale 1 add >scale
        end
        magnitude 1024 ge
    end
    while
        magnitude 512 lt if
            magnitude 1 shl >magnitude
            scale 1 sub >scale
        end
        magnitude 512 lt
    end
    magnitude 512 sub 14 shl
    scale 136 add 23 shl or sign or ret
;

# Integer approximation matching the rotating cube demo's angle math.
fn sin_deg (angle) :
    local a
    local sign 1
    local t
    angle 360 mod >a
    a 0 lt if
        a 360 add >a
    end
    a 180 ge if
        a 180 sub >a
        -1 >sign
    end
    a 180 a sub mul >t
    t 4 mul 1024 mul 40500 t sub div sign mul ret
;

# Orbit the camera by updating VIEW while the model stays fixed at the origin.
fn set_view_matrix (yaw pitch) :
    local sy
    local cy
    local sx
    local cx
    local m10
    local m12
    local m20
    local m22
    yaw sin_deg >sy
    yaw 90 add sin_deg >cy
    pitch sin_deg >sx
    pitch 90 add sin_deg >cx
    sx sy mul 1024 div >m10
    sx cy mul neg 1024 div >m12
    cx sy mul neg 1024 div >m20
    cx cy mul 1024 div >m22

    0x02000011 emit_word
    1 emit_word # VIEW
    cy q10_to_f32 emit_word 0.0f emit_word sy q10_to_f32 emit_word 0.0f emit_word
    m10 q10_to_f32 emit_word cx q10_to_f32 emit_word m12 q10_to_f32 emit_word 0.0f emit_word
    m20 q10_to_f32 emit_word sx q10_to_f32 emit_word m22 q10_to_f32 emit_word 3.5f emit_word
    0.0f emit_word 0.0f emit_word 0.0f emit_word 1.0f emit_word
;

fn clear_frame () :
    0x04000003 emit_word
    3 emit_word
    0x101018FF emit_word
    1.0f emit_word
;
''' if rotate else ""
    if rotate:
        dmac_directives = '''
!const DMAC_GLOBAL_CONTROL 0x80050004
!const DMAC_CHANNEL_ENABLE 0x80050008
!const DMAC_CH0_SRC 0x80050100
!const DMAC_CH0_DST 0x80050104
!const DMAC_CH0_COUNT 0x80050108
!const DMAC_CH0_CONTROL 0x8005010C
!const DMAC_CH0_STATUS 0x80050110
'''
        dmac_helpers = '''

# Transfer one contiguous RAM command buffer to the VPU FIFO through DMAC.
fn submit_vpu_commands (source byte_count) :
    6 DMAC_CH0_STATUS st
    1 DMAC_GLOBAL_CONTROL st
    1 DMAC_CHANNEL_ENABLE st
    source DMAC_CH0_SRC st
    VPU_FIFO_DATA DMAC_CH0_DST st
    byte_count DMAC_CH0_COUNT st
    0x112 DMAC_CH0_CONTROL st # 32-bit, increment source, fixed FIFO destination, START
    while
        DMAC_CH0_STATUS ld 1 and 0 neq
    end
    DMAC_CH0_STATUS ld 2 and 0 eq if
        halt
    end
;
'''
        irq_registers = "!var IRQC_PENDING 0xFFFF0040\n!var IRQC_ENABLE 0xFFFF0044"
        irq_setup = "    1 IRQC_PENDING sth\n    1 IRQC_ENABLE sth"
        draw_loop = '''
    local yaw 25
    local pitch 20
    while
        clear_frame
        yaw pitch set_view_matrix
        0 draw_cached_model
        yaw 2 add 360 mod >yaw
        pitch 3 add 360 mod >pitch
        halt
        0
    end'''
    else:
        dmac_directives = ""
        dmac_helpers = ""
        irq_registers = ""
        irq_setup = ""
        draw_loop = "    MODEL_DATA_ADDR MODEL_TRIANGLE_COUNT draw_triangles"
    initial_commands = (
        "TEXTURE_BLOB_ADDR MODEL_DMA_BYTES submit_vpu_commands"
        if rotate else
        "TEXTURE_BLOB_ADDR TEXTURE_BLOB_WORDS emit_blob\n    MODEL_BLOB_ADDR MODEL_SETUP_WORDS emit_blob"
    )
    return f'''# Generated by obj_to_sol.py from {model.source_name if hasattr(model, 'source_name') else 'OBJ'}.
# Float attributes are stored as IEEE-754 binary32 bit patterns in the blobs.
!stack_top 0x000FFFFC
!var_base 0x00100000
!rodata_base 0x{data_base:08X}
!const VPU_FIFO_DATA 0x{VPU_FIFO_DATA:08X}
!const VPU_STATUS 0x{VPU_STATUS:08X}
!const VDP_ENABLE 0x{VDP_ENABLE:08X}
!const VDP_MODE 0x{VDP_MODE:08X}
!const VDP_BORDER 0x{VDP_BORDER:08X}
{irq_registers}
{dmac_directives}
!const SHADE_MODE 2 # 1=flat, 2=Gouraud
!const TEXTURE_BLOB_ADDR 0x{data_base:08X}
!const TEXTURE_BLOB_WORDS {len(texture_words)}
!const MODEL_BLOB_ADDR 0x{model_base:08X}
!const MODEL_SETUP_WORDS {setup_words}
!const MODEL_DATA_ADDR 0x{model_data_base:08X}
!const MODEL_TRIANGLE_COUNT {triangles}
!const MODEL_DMA_BYTES {dma_total_bytes}

{data_block(texture_words)}

{data_block(model_words)}

fn emit_blob (_ptr count) :
    local ptr 0
    local i 0
    _ptr >ptr
    while
        while
            VPU_STATUS ld 2 and 0 neq
        end
        ptr ld VPU_FIFO_DATA st
        ptr 4 add >ptr
        i 1 add >i
        i count lt
    end
;

fn emit_word (word) :
    while
        VPU_STATUS ld 2 and 0 neq
    end
    word VPU_FIFO_DATA st
;

fn draw_triangles (_ptr count) :
    local ptr 0
    local i 0
    local j
    _ptr >ptr
    while
        0x{TEXTURED_LIT_TRIANGLE_OPCODE:02X}00001D emit_word
        ptr ld emit_word # texture ID
        SHADE_MODE emit_word
        ptr 4 add >ptr
        0 >j
        while
            ptr ld emit_word
            ptr 4 add >ptr
            j 1 add >j
            j 27 lt
        end
        i 1 add >i
        i count lt
    end
;
{rotation_helpers}
{dmac_helpers}

fn draw_cached_model (model_id) :
    0x1B000002 emit_word
    model_id emit_word
    SHADE_MODE emit_word
;

fn main () :
    1 VDP_ENABLE stb
    0 VDP_MODE stb
    0 VDP_BORDER stb
    {initial_commands}
{irq_setup}
{draw_loop}
;

main
'''


def convert(obj_path: Path, output_path: Path, data_base: int = 0x00020000, rotate: bool = False) -> tuple[int, int, int]:
    model = parse_obj(obj_path)
    model.source_name = obj_path.name
    materials = parse_mtl(model.mtllibs)
    texture_words, resolved_materials, texture_count = texture_commands(model, materials)
    draw_words, triangle_count, setup_words = model_commands(model, resolved_materials)
    source = generate_sol(model, texture_words, draw_words, setup_words, triangle_count, data_base, rotate)
    output_path.write_text(source, encoding="utf-8")
    return triangle_count, texture_count, len(texture_words) + len(draw_words)


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description="Convert OBJ + MTL diffuse textures into a self-contained VPU sol program.")
    parser.add_argument("input", type=Path, help="input Wavefront .obj file")
    parser.add_argument("-o", "--output", type=Path, help="output .sol path (default: input name with .sol extension)")
    parser.add_argument("--data-base", type=lambda s: int(s, 0), default=0x00020000,
                        help="read-only data address for embedded blobs (default: 0x20000)")
    parser.add_argument("--rotate", action="store_true",
                        help="orbit the camera and cache the model; initial commands are sent through DMAC")
    return parser


def main(argv: list[str] | None = None) -> int:
    args = build_parser().parse_args(argv)
    output = args.output or args.input.with_suffix(".sol")
    try:
        triangles, textures, words = convert(args.input, output, args.data_base, args.rotate)
    except (ObjConvertError, OSError) as exc:
        print(f"obj_to_sol: error: {exc}", file=sys.stderr)
        return 1
    print(f"Wrote {output}: {triangles} triangles, {textures} texture(s), {words} blob words")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
