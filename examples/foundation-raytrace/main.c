#include <foundation/fb.h>
#include <foundation/sys.h>

enum { ONE = 1024, NEAR = 12, FAR = 24 * ONE };
enum { WIDTH = 320, HEIGHT = 240, FIRST_COLOR = 64, SHADES = 32 };
enum { SKY, RED, BLUE, FLOOR_LIGHT, FLOOR_DARK, MATERIALS, MIRROR };

typedef struct { int32_t x, y, z; } vec;
typedef struct { vec center; int32_t radius; int material; } sphere;

static const sphere scene[] = {
    {{-1400, -224, 4300},  800, RED},
    {{  200,  176, 5200}, 1200, MIRROR},
    {{ 1600, -374, 3750},  650, BLUE},
};
static const vec light = {-512, 768, -444};
static const uint8_t colors[MATERIALS][3] = {
    {134, 188, 243}, {250, 85, 57}, {64, 169, 249}, {226, 216, 182}, {90, 96, 105},
};

static vec add(vec a, vec b) {
    return (vec){a.x + b.x, a.y + b.y, a.z + b.z};
}

static vec sub(vec a, vec b) {
    return (vec){a.x - b.x, a.y - b.y, a.z - b.z};
}

static vec scale(vec a, int32_t k) {
    return (vec){a.x * k / ONE, a.y * k / ONE, a.z * k / ONE};
}

static int32_t dot(vec a, vec b) {
    return (a.x * b.x + a.y * b.y + a.z * b.z) / ONE;
}

static int32_t isqrt(uint32_t n) {
    uint32_t root = 0;
    for (uint32_t bit = 1u << 30; bit != 0; bit >>= 2) {
        if (n >= root + bit) {
            n -= root + bit;
            root = (root >> 1) + bit;
        } else {
            root >>= 1;
        }
    }
    return (int32_t)root;
}

/* Solve a*t*t + 2*b*t + c = 0 for an arbitrary-length direction. */
static int32_t sphere_distance(vec origin, vec dir, const sphere *s) {
    vec offset = sub(origin, s->center);
    int32_t a = dot(dir, dir);
    int32_t b = dot(dir, offset);
    int32_t c = dot(offset, offset) - s->radius * s->radius / ONE;
    int32_t discriminant = b * b - a * c;
    if (discriminant < 0) return FAR;
    int32_t root = isqrt((uint32_t)discriminant);
    int32_t t = (-b - root) * ONE / a;
    return t > NEAR ? t : (-b + root) * ONE / a;
}

static uint8_t shade(int material, int32_t level) {
    if (level < 0) level = 0;
    if (level > ONE) level = ONE;
    return (uint8_t)(FIRST_COLOR + material * SHADES + level * (SHADES - 1) / ONE);
}

static uint8_t trace(vec dir) {
    vec origin = {0, 256, 0};
    for (int bounce = 0; ; ++bounce) {
        int32_t closest = FAR;
        const sphere *hit = 0;
        if (dir.y < 0) {
            int32_t t = (-ONE - origin.y) * ONE / dir.y;
            if (t > NEAR && t < closest) closest = t;
        }
        for (unsigned i = 0; i < sizeof scene / sizeof scene[0]; ++i) {
            int32_t t = sphere_distance(origin, dir, &scene[i]);
            if (t > NEAR && t < closest) {
                closest = t;
                hit = &scene[i];
            }
        }
        if (closest == FAR) return shade(SKY, 530 + dir.y / 2);

        vec point = add(origin, scale(dir, closest));
        vec normal = {0, ONE, 0};
        int material;
        if (hit) {
            normal = scale(sub(point, hit->center), ONE * ONE / hit->radius);
            if (hit->material == MIRROR) {
                if (bounce != 0) return shade(SKY, 280);
                dir = sub(dir, scale(normal, 2 * dot(dir, normal)));
                origin = add(point, scale(normal, NEAR));
                continue;
            }
            material = hit->material;
        } else {
            /* Bias to positive coordinates so the checker also floors x < 0. */
            int32_t square = (point.x + 32 * ONE) / ONE + point.z / ONE;
            material = square % 2 == 0 ? FLOOR_LIGHT : FLOOR_DARK;
        }

        int32_t diffuse = dot(normal, light);
        if (diffuse > 0) {
            vec shadow_origin = add(point, scale(normal, NEAR));
            for (unsigned i = 0; i < sizeof scene / sizeof scene[0]; ++i) {
                int32_t t = sphere_distance(shadow_origin, light, &scene[i]);
                if (t > NEAR && t < FAR) {
                    diffuse = 0;
                    break;
                }
            }
        } else {
            diffuse = 0;
        }
        return shade(material, 220 + diffuse * 3 / 4);
    }
}

static fnd_status draw(fnd_handle fb) {
    fnd_fb_info info;
    fnd_status st = fnd_fb_get_info(fb, &info);
    if (st != FND_OK) return st;
    if (info.format != FND_FB_INDEX8 || info.width != WIDTH ||
        info.height != HEIGHT || info.stride != WIDTH ||
        info.clut_entries < FIRST_COLOR + MATERIALS * SHADES) return FND_E_UNSUPPORTED;

    uint8_t palette[MATERIALS * SHADES][3];
    for (int m = 0; m < MATERIALS; ++m) {
        for (int s = 0; s < SHADES; ++s) {
            for (int c = 0; c < 3; ++c) {
                palette[m * SHADES + s][c] = (uint8_t)(colors[m][c] * s / (SHADES - 1));
            }
        }
    }
    st = fnd_fb_clut_write(fb, FIRST_COLOR, MATERIALS * SHADES, palette);
    if (st != FND_OK) return st;

    /* Trace one ray per 2x2 block and present two scanlines at a time. */
    uint8_t rows[2][WIDTH];
    for (int y = 0; y < HEIGHT; y += 2) {
        for (int x = 0; x < WIDTH; x += 2) {
            vec dir = {(x + 1 - WIDTH / 2) * 5, (HEIGHT / 2 - y - 1) * 5 - 140, ONE};
            uint8_t pixel = trace(dir);
            rows[0][x] = rows[0][x + 1] = rows[1][x] = rows[1][x + 1] = pixel;
        }
        const uint8_t *data = (const uint8_t *)rows;
        for (uint32_t offset = 0; offset < sizeof rows; ) {
            uint32_t written;
            st = fnd_write(fb, data + offset, sizeof rows - offset, &written);
            if (st != FND_OK) return st;
            if (written == 0) return FND_E_IO;
            offset += written;
        }
    }
    return FND_OK;
}

int main(void) {
    fnd_init();
    fnd_handle fb;
    fnd_status st = fnd_file_open("/dev/fb0", FND_OPEN_WRITE, FND_MODE_DEFAULT, &fb);
    if (st != FND_OK) return st;
    st = draw(fb);
    if (st == FND_OK) {
        int32_t hz = fnd_sys_time_get(FND_TIME_TICK_HZ);
        if (hz > 0) fnd_sys_sleep((uint32_t)hz * 10);
    }
    fnd_status closed = fnd_close(fb);
    return st == FND_OK ? closed : st;
}
