#include "libraw.h"
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

/* ── Panasonic AFPointPosition (tag 0x004d) 捕获 ─────────────────────
 * libraw 的 makernotes_cb 在解析每个 makernote tag 前调用（tag 已 OR
 * uptag<<16；Panasonic 主 IFD 的 uptag = 0x30000）。0x004d 值为
 * rational64u×2（X/Y 已归一化 0–1；哨兵 16777216/16777216 = none）。
 * 使用 per-call context（set_makernotes_handler 的 datap），无全局状态，
 * 支持并行 EXIF 提取。 */
typedef struct
{
    int valid;
    unsigned vals[4]; /* x_num, x_den, y_num, y_den */
} rawlib_pan_ctx_t;

static void rawlib_panasonic_makernotes_cb(void *context, int tag, int type,
                                           int len, unsigned ord, void *ifp,
                                           INT64 base)
{
    (void)type;
    (void)base;
    rawlib_pan_ctx_t *c = (rawlib_pan_ctx_t *)context;
    if (!c)
        return;
    if ((unsigned)tag != (0x4d | 0x30000))
        return;
    if (len < 2) /* 2 个 rational64u（条目数） */
        return;
    FILE *f = (FILE *)ifp;
    unsigned vals[4];
    unsigned char b[4];
    for (int i = 0; i < 4; i++)
    {
        if (fread(b, 1, 4, f) != 4)
            return;
        vals[i] = (ord == 0x4d4d)
            ? ((unsigned)b[0] << 24) | ((unsigned)b[1] << 16) |
                  ((unsigned)b[2] << 8) | b[3]
            : b[0] | ((unsigned)b[1] << 8) | ((unsigned)b[2] << 16) |
                  ((unsigned)b[3] << 24);
    }
    for (int i = 0; i < 4; i++)
        c->vals[i] = vals[i];
    c->valid = 1;
}

void *rawlib_pan_ctx_alloc(void)
{
    return calloc(1, sizeof(rawlib_pan_ctx_t));
}

void rawlib_pan_ctx_free(void *ctx)
{
    free(ctx);
}

/* 在 libraw_open_* 之前调用：注册回调并清空捕获状态 */
void rawlib_panasonic_af_init(libraw_data_t *data, void *ctx)
{
    rawlib_pan_ctx_t *c = (rawlib_pan_ctx_t *)ctx;
    if (c)
        c->valid = 0;
    libraw_set_makernotes_handler(data, rawlib_panasonic_makernotes_cb, ctx);
}

/* open 之后调用：取归一化坐标（0–1）；无记录/哨兵/越界返回 0 */
int rawlib_panasonic_af_get(void *ctx, double *out_x, double *out_y)
{
    rawlib_pan_ctx_t *c = (rawlib_pan_ctx_t *)ctx;
    if (!c || !c->valid)
        return 0;
    /* none 哨兵：16777216/16777216 */
    if (c->vals[0] == 0x1000000 && c->vals[1] == 0x1000000)
        return 0;
    if (c->vals[1] == 0 || c->vals[3] == 0)
        return 0;
    double x = (double)c->vals[0] / (double)c->vals[1];
    double y = (double)c->vals[2] / (double)c->vals[3];
    if (x < 0.0 || x > 1.0 || y < 0.0 || y > 1.0)
        return 0;
    *out_x = x;
    *out_y = y;
    return 1;
}

/* 大小写不敏感地检查 make 是否含 "fuji"（相机 make 实际为 "Fujifilm" 等） */
static int make_is_fuji(const char *make)
{
    if (!make)
        return 0;
    const char *s = make;
    while (*s)
    {
        if ((*s == 'f' || *s == 'F') &&
            (s[1] == 'u' || s[1] == 'U') &&
            (s[2] == 'j' || s[2] == 'J') &&
            (s[3] == 'i' || s[3] == 'I'))
            return 1;
        s++;
    }
    return 0;
}

/*
 * 读取 Fuji makernotes 的对焦点像素坐标（FocusPixel，tag 0x1023）。
 *
 * 语义：
 * - 坐标相对未旋转的原始传感器图像（JPEG 提取路径在 photo-engine 层按
 *   EXIF orientation 统一旋转到显示方向）。
 * - makernotes 是 struct（非 union），fuji 分支仅 Fuji 机型解析器填充，
 *   其他厂商不会写入；双保险：make 含 "FUJI" 且坐标非 0。
 * - 返回 1 = 有效，0 = 无记录。
 */
int rawlib_get_focus_pixel(libraw_data_t *data,
                           unsigned short *out_x,
                           unsigned short *out_y)
{
    if (!data)
        return 0;
    unsigned short x = data->makernotes.fuji.FocusPixel[0];
    unsigned short y = data->makernotes.fuji.FocusPixel[1];
    if (x == 0 && y == 0)
        return 0;
    if (!make_is_fuji(data->idata.make))
        return 0;
    *out_x = x;
    *out_y = y;
    return 1;
}

/*
 * 读取 makernotes 的 AFInfo/AFInfo2 原始 blob（Nikon 0x0088/0x00b7、
 * Panasonic 0x004d 等，libraw 只存不解析）。返回 1 = 有数据，0 = 无。
 * out_buf 由调用方分配，容量 buf_cap；实际长度写入 *out_len。
 * order = TIFF 字节序（0x4949 小端 / 0x4d4d 大端）；version = AFInfo2 版本号
 * （300 = 0300 等；AFInfo 老版未设置时为 0）。
 */
int rawlib_get_afinfo(libraw_data_t *data,
                      unsigned *out_tag,
                      short *out_order,
                      unsigned *out_version,
                      unsigned *out_len,
                      unsigned char *out_buf,
                      unsigned buf_cap)
{
    if (!data || !out_tag || !out_order || !out_version || !out_len || !out_buf)
        return 0;
    if (data->makernotes.common.afcount <= 0)
        return 0;
    libraw_afinfo_item_t *item = &data->makernotes.common.afdata[0];
    if (!item->AFInfoData || item->AFInfoData_length == 0)
        return 0;
    *out_tag = item->AFInfoData_tag;
    *out_order = item->AFInfoData_order;
    *out_version = item->AFInfoData_version;
    *out_len = item->AFInfoData_length;
    unsigned n = item->AFInfoData_length < buf_cap ? item->AFInfoData_length : buf_cap;
    memcpy(out_buf, item->AFInfoData, n);
    return 1;
}
