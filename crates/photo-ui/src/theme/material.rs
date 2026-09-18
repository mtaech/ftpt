//! Material You 色彩引擎：CAM16 + HCT + 色调板。移植自 @material/material-color-utilities。
//!
//! 移植来源（逐行对照翻译，未凭记忆改写公式）：@material/material-color-utilities@0.4.0 的
//! hct/cam16.js、hct/hct.js、hct/hct_solver.js、hct/viewing_conditions.js、
//! utils/color_utils.js、utils/math_utils.js、utils/string_utils.js、palettes/tonal_palette.js。
//!
//! 不变量：
//! - 全部采用 f64 双精度运算，与 TypeScript 原版的 IEEE-754 double 语义保持一致；
//! - ARGB 一律视为不透明的 0xFFRRGGBB（u32），alpha 被忽略；
//! - 默认观察条件 DEFAULT 对应 viewing_conditions.js 中 ViewingConditions.make() 的无参调用结果；
//! - 仅依赖 std，不使用 unsafe，不引入任何外部 crate。

// ─────────────────────────────── math_utils ───────────────────────────────

/// 符号函数：正数返回 1，负数返回 -1，零返回 0。
fn signum(num: f64) -> f64 {
    if num < 0.0 {
        -1.0
    } else if num == 0.0 {
        0.0
    } else {
        1.0
    }
}

/// 线性插值：amount 为 0 返回 start，为 1 返回 stop。
fn lerp(start: f64, stop: f64, amount: f64) -> f64 {
    (1.0 - amount) * start + amount * stop
}

/// 将角度规范化到 [0.0, 360.0)。
fn sanitize_degrees_double(degrees: f64) -> f64 {
    let mut degrees = degrees % 360.0;
    if degrees < 0.0 {
        degrees += 360.0;
    }
    degrees
}

/// 1x3 行向量与 3x3 矩阵相乘。
fn matrix_multiply(row: [f64; 3], matrix: [[f64; 3]; 3]) -> [f64; 3] {
    [
        row[0] * matrix[0][0] + row[1] * matrix[0][1] + row[2] * matrix[0][2],
        row[0] * matrix[1][0] + row[1] * matrix[1][1] + row[2] * matrix[1][2],
        row[0] * matrix[2][0] + row[1] * matrix[2][1] + row[2] * matrix[2][2],
    ]
}

// ─────────────────────────────── color_utils ───────────────────────────────

/// sRGB 到 XYZ 的转换矩阵。
const SRGB_TO_XYZ: [[f64; 3]; 3] = [
    [0.41233895, 0.35762064, 0.18051042],
    [0.2126, 0.7152, 0.0722],
    [0.01932141, 0.11916382, 0.95034478],
];

/// D65 白点（晴天正午）。
const WHITE_POINT_D65: [f64; 3] = [95.047, 100.0, 108.883];

/// 由 RGB 分量构造不透明 ARGB。
fn argb_from_rgb(red: u8, green: u8, blue: u8) -> u32 {
    0xff00_0000 | ((red as u32) << 16) | ((green as u32) << 8) | blue as u32
}

/// 取 ARGB 的红色分量。
fn red_from_argb(argb: u32) -> u8 {
    ((argb >> 16) & 0xff) as u8
}

/// 取 ARGB 的绿色分量。
fn green_from_argb(argb: u32) -> u8 {
    ((argb >> 8) & 0xff) as u8
}

/// 取 ARGB 的蓝色分量。
fn blue_from_argb(argb: u32) -> u8 {
    (argb & 0xff) as u8
}

/// 解析两位十六进制为一个字节。
fn parse_hex_pair(high: u8, low: u8) -> Option<u8> {
    fn nibble(byte: u8) -> Option<u8> {
        match byte {
            b'0'..=b'9' => Some(byte - b'0'),
            b'a'..=b'f' => Some(byte - b'a' + 10),
            b'A'..=b'F' => Some(byte - b'A' + 10),
            _ => None,
        }
    }
    Some((nibble(high)? << 4) | nibble(low)?)
}

/// "#RRGGBB" 或 "RRGGBB" -> ARGB（不透明）。非法返回 None。
///
/// 为与原版 string_utils.argbFromHex 对齐，同时接受 3 位与 8 位（8 位的前两位 alpha 被忽略）；
/// 原版对非法字符依赖 parseInt 的 NaN 静默归零，这里改为返回 None。
pub fn argb_from_hex(hex: &str) -> Option<u32> {
    let hex = hex.strip_prefix('#').unwrap_or(hex);
    let bytes = hex.as_bytes();
    let (r, g, b) = match bytes.len() {
        3 => (
            parse_hex_pair(bytes[0], bytes[0])?,
            parse_hex_pair(bytes[1], bytes[1])?,
            parse_hex_pair(bytes[2], bytes[2])?,
        ),
        6 => (
            parse_hex_pair(bytes[0], bytes[1])?,
            parse_hex_pair(bytes[2], bytes[3])?,
            parse_hex_pair(bytes[4], bytes[5])?,
        ),
        8 => (
            parse_hex_pair(bytes[2], bytes[3])?,
            parse_hex_pair(bytes[4], bytes[5])?,
            parse_hex_pair(bytes[6], bytes[7])?,
        ),
        _ => return None,
    };
    Some(0xff00_0000 | ((r as u32) << 16) | ((g as u32) << 8) | b as u32)
}

/// ARGB -> "#rrggbb"（小写，忽略 alpha）。
pub fn hex_from_argb(argb: u32) -> String {
    format!(
        "#{:02x}{:02x}{:02x}",
        red_from_argb(argb),
        green_from_argb(argb),
        blue_from_argb(argb)
    )
}

/// 将 sRGB 单通道线性化：0..255 -> 0..100。
fn linearized(rgb_component: u8) -> f64 {
    let normalized = rgb_component as f64 / 255.0;
    if normalized <= 0.040449936 {
        normalized / 12.92 * 100.0
    } else {
        ((normalized + 0.055) / 1.055).powf(2.4) * 100.0
    }
}

/// 将线性分量反线性化并四舍五入到 0..255。
fn delinearized(rgb_component: f64) -> u8 {
    let normalized = rgb_component / 100.0;
    let value = if normalized <= 0.0031308 {
        normalized * 12.92
    } else {
        1.055 * normalized.powf(1.0 / 2.4) - 0.055
    };
    (value * 255.0).round().clamp(0.0, 255.0) as u8
}

/// Lab 的 f 函数。
fn lab_f(t: f64) -> f64 {
    let e = 216.0 / 24389.0;
    let kappa = 24389.0 / 27.0;
    if t > e {
        t.powf(1.0 / 3.0)
    } else {
        (kappa * t + 16.0) / 116.0
    }
}

/// Lab 的 f 逆函数。
fn lab_invf(ft: f64) -> f64 {
    let e = 216.0 / 24389.0;
    let kappa = 24389.0 / 27.0;
    let ft3 = ft * ft * ft;
    if ft3 > e {
        ft3
    } else {
        (116.0 * ft - 16.0) / kappa
    }
}

/// 由线性 RGB 构造不透明 ARGB。
fn argb_from_linrgb(linrgb: [f64; 3]) -> u32 {
    argb_from_rgb(
        delinearized(linrgb[0]),
        delinearized(linrgb[1]),
        delinearized(linrgb[2]),
    )
}

/// 由 ARGB 计算 XYZ。
fn xyz_from_argb(argb: u32) -> [f64; 3] {
    let r = linearized(red_from_argb(argb));
    let g = linearized(green_from_argb(argb));
    let b = linearized(blue_from_argb(argb));
    matrix_multiply([r, g, b], SRGB_TO_XYZ)
}

/// 由 L* 构造灰度 ARGB。
fn argb_from_lstar(lstar: f64) -> u32 {
    let y = y_from_lstar(lstar);
    let component = delinearized(y);
    argb_from_rgb(component, component, component)
}

/// 由 ARGB 计算 L*。
fn lstar_from_argb(argb: u32) -> f64 {
    let y = xyz_from_argb(argb)[1];
    116.0 * lab_f(y / 100.0) - 16.0
}

/// 由 L* 计算 Y。
fn y_from_lstar(lstar: f64) -> f64 {
    100.0 * lab_invf((lstar + 16.0) / 116.0)
}

// ─────────────────────────── viewing_conditions ───────────────────────────

/// CAM16 转换过程中只依赖观察条件的中间量缓存。
#[derive(Debug, Clone)]
struct ViewingConditions {
    /// 背景亮度因子 Y / Yw。
    n: f64,
    /// 白色响应。
    aw: f64,
    nbb: f64,
    ncb: f64,
    /// 环境指数。
    c: f64,
    nc: f64,
    /// 色适应后的 RGB 缩放因子。
    rgb_d: [f64; 3],
    fl: f64,
    z: f64,
}

impl ViewingConditions {
    /// 由物理参数构造观察条件（对应原版 ViewingConditions.make）。
    fn make(
        white_point: [f64; 3],
        adapting_luminance: f64,
        background_lstar: f64,
        surround: f64,
        discounting_illuminant: bool,
    ) -> Self {
        let xyz = white_point;
        let r_w = xyz[0] * 0.401288 + xyz[1] * 0.650173 + xyz[2] * -0.051461;
        let g_w = xyz[0] * -0.250268 + xyz[1] * 1.204414 + xyz[2] * 0.045854;
        let b_w = xyz[0] * -0.002079 + xyz[1] * 0.048952 + xyz[2] * 0.953127;
        let f = 0.8 + surround / 10.0;
        let c = if f >= 0.9 {
            lerp(0.59, 0.69, (f - 0.9) * 10.0)
        } else {
            lerp(0.525, 0.59, (f - 0.8) * 10.0)
        };
        let mut d = if discounting_illuminant {
            1.0
        } else {
            f * (1.0 - (1.0 / 3.6) * ((-adapting_luminance - 42.0) / 92.0).exp())
        };
        d = d.clamp(0.0, 1.0);
        let nc = f;
        let rgb_d = [
            d * (100.0 / r_w) + 1.0 - d,
            d * (100.0 / g_w) + 1.0 - d,
            d * (100.0 / b_w) + 1.0 - d,
        ];
        let k = 1.0 / (5.0 * adapting_luminance + 1.0);
        let k4 = k * k * k * k;
        let k4_f = 1.0 - k4;
        let fl = k4 * adapting_luminance + 0.1 * k4_f * k4_f * (5.0 * adapting_luminance).cbrt();
        let n = y_from_lstar(background_lstar) / white_point[1];
        let z = 1.48 + n.sqrt();
        let nbb = 0.725 / n.powf(0.2);
        let ncb = nbb;
        let rgb_a_factors = [
            (fl * rgb_d[0] * r_w / 100.0).powf(0.42),
            (fl * rgb_d[1] * g_w / 100.0).powf(0.42),
            (fl * rgb_d[2] * b_w / 100.0).powf(0.42),
        ];
        let rgb_a = [
            (400.0 * rgb_a_factors[0]) / (rgb_a_factors[0] + 27.13),
            (400.0 * rgb_a_factors[1]) / (rgb_a_factors[1] + 27.13),
            (400.0 * rgb_a_factors[2]) / (rgb_a_factors[2] + 27.13),
        ];
        let aw = (2.0 * rgb_a[0] + rgb_a[1] + 0.05 * rgb_a[2]) * nbb;
        Self {
            n,
            aw,
            nbb,
            ncb,
            c,
            nc,
            rgb_d,
            fl,
            z,
        }
    }
}

/// 默认（类 sRGB）观察条件，等价于原版 ViewingConditions.DEFAULT。
///
/// 原版还缓存了 fLRoot，本模块只用到 hue / chroma，故未保留该字段。
fn default_viewing_conditions() -> &'static ViewingConditions {
    use std::sync::OnceLock;
    static DEFAULT: OnceLock<ViewingConditions> = OnceLock::new();
    DEFAULT.get_or_init(|| {
        let adapting_luminance = (200.0 / std::f64::consts::PI) * y_from_lstar(50.0) / 100.0;
        ViewingConditions::make(WHITE_POINT_D65, adapting_luminance, 50.0, 2.0, false)
    })
}

// ─────────────────────────────── cam16 ───────────────────────────────

/// CAM16 颜色在默认观察条件下的结果（本模块只用到 hue 与 chroma）。
#[derive(Debug, Clone, Copy)]
struct Cam16 {
    hue: f64,
    chroma: f64,
}

/// 由 ARGB 计算默认观察条件下的 CAM16 hue 与 chroma（对应原版 Cam16.fromInt）。
fn cam16_from_argb(argb: u32) -> Cam16 {
    let vc = default_viewing_conditions();
    let red_l = linearized(red_from_argb(argb));
    let green_l = linearized(green_from_argb(argb));
    let blue_l = linearized(blue_from_argb(argb));
    let x = 0.41233895 * red_l + 0.35762064 * green_l + 0.18051042 * blue_l;
    let y = 0.2126 * red_l + 0.7152 * green_l + 0.0722 * blue_l;
    let z = 0.01932141 * red_l + 0.11916382 * green_l + 0.95034478 * blue_l;
    let r_c = 0.401288 * x + 0.650173 * y - 0.051461 * z;
    let g_c = -0.250268 * x + 1.204414 * y + 0.045854 * z;
    let b_c = -0.002079 * x + 0.048952 * y + 0.953127 * z;
    let r_d = vc.rgb_d[0] * r_c;
    let g_d = vc.rgb_d[1] * g_c;
    let b_d = vc.rgb_d[2] * b_c;
    let r_af = (vc.fl * r_d.abs() / 100.0).powf(0.42);
    let g_af = (vc.fl * g_d.abs() / 100.0).powf(0.42);
    let b_af = (vc.fl * b_d.abs() / 100.0).powf(0.42);
    let r_a = (signum(r_d) * 400.0 * r_af) / (r_af + 27.13);
    let g_a = (signum(g_d) * 400.0 * g_af) / (g_af + 27.13);
    let b_a = (signum(b_d) * 400.0 * b_af) / (b_af + 27.13);
    let a = (11.0 * r_a + -12.0 * g_a + b_a) / 11.0;
    let b = (r_a + g_a - 2.0 * b_a) / 9.0;
    let u = (20.0 * r_a + 20.0 * g_a + 21.0 * b_a) / 20.0;
    let p2 = (40.0 * r_a + 20.0 * g_a + b_a) / 20.0;
    let atan_degrees = b.atan2(a) * 180.0 / std::f64::consts::PI;
    let hue = sanitize_degrees_double(atan_degrees);
    let ac = p2 * vc.nbb;
    let j = 100.0 * (ac / vc.aw).powf(vc.c * vc.z);
    let hue_prime = if hue < 20.14 { hue + 360.0 } else { hue };
    let e_hue = 0.25 * (((hue_prime * std::f64::consts::PI) / 180.0 + 2.0).cos() + 3.8);
    let p1 = (50000.0 / 13.0) * e_hue * vc.nc * vc.ncb;
    let t = (p1 * (a * a + b * b).sqrt()) / (u + 0.305);
    let alpha = t.powf(0.9) * (1.64 - 0.29f64.powf(vc.n)).powf(0.73);
    let chroma = alpha * (j / 100.0).sqrt();
    Cam16 { hue, chroma }
}

// ─────────────────────────────── hct_solver ───────────────────────────────

/// 线性 RGB 到缩放色适应响应的矩阵（原版 SCALED_DISCOUNT_FROM_LINRGB）。
const SCALED_DISCOUNT_FROM_LINRGB: [[f64; 3]; 3] = [
    [
        0.001200833568784504,
        0.002389694492170889,
        0.0002795742885861124,
    ],
    [
        0.0005891086651375999,
        0.0029785502573438758,
        0.0003270666104008398,
    ],
    [
        0.00010146692491640572,
        0.0005364214359186694,
        0.0032979401770712076,
    ],
];

/// 缩放色适应响应回线性 RGB 的矩阵（原版 LINRGB_FROM_SCALED_DISCOUNT）。
const LINRGB_FROM_SCALED_DISCOUNT: [[f64; 3]; 3] = [
    [1373.2198709594231, -1100.4251190754821, -7.278681089101213],
    [-271.815969077903, 559.6580465940733, -32.46047482791194],
    [1.9622899599665666, -57.173814538844006, 308.7233197812385],
];

/// 线性 RGB 各通道对 Y 的权重。
const Y_FROM_LINRGB: [f64; 3] = [0.2126, 0.7152, 0.0722];

/// sRGB 临界平面（原版 CRITICAL_PLANES）。
const CRITICAL_PLANES: &[f64] = &[
    0.015176349177441876,
    0.045529047532325624,
    0.07588174588720938,
    0.10623444424209313,
    0.13658714259697685,
    0.16693984095186062,
    0.19729253930674434,
    0.2276452376616281,
    0.2579979360165119,
    0.28835063437139563,
    0.3188300904430532,
    0.350925934958123,
    0.3848314933096426,
    0.42057480301049466,
    0.458183274052838,
    0.4976837250274023,
    0.5391024159806381,
    0.5824650784040898,
    0.6277969426914107,
    0.6751227633498623,
    0.7244668422128921,
    0.775853049866786,
    0.829304845476233,
    0.8848452951698498,
    0.942497089126609,
    1.0022825574869039,
    1.0642236851973577,
    1.1283421258858297,
    1.1946592148522128,
    1.2631959812511864,
    1.3339731595349034,
    1.407011200216447,
    1.4823302800086415,
    1.5599503113873272,
    1.6398909516233677,
    1.7221716113234105,
    1.8068114625156377,
    1.8938294463134073,
    1.9832442801866852,
    2.075074464868551,
    2.1693382909216234,
    2.2660538449872063,
    2.36523901573795,
    2.4669114995532007,
    2.5710888059345764,
    2.6777882626779785,
    2.7870270208169257,
    2.898822059350997,
    3.0131901897720907,
    3.1301480604002863,
    3.2497121605402226,
    3.3718988244681087,
    3.4967242352587946,
    3.624204428461639,
    3.754355295633311,
    3.887192587735158,
    4.022731918402185,
    4.160988767090289,
    4.301978482107941,
    4.445716283538092,
    4.592217266055746,
    4.741496401646282,
    4.893568542229298,
    5.048448422192488,
    5.20615066083972,
    5.3666897647573375,
    5.5300801301023865,
    5.696336044816294,
    5.865471690767354,
    6.037501145825082,
    6.212438385869475,
    6.390297286737924,
    6.571091626112461,
    6.7548350853498045,
    6.941541251256611,
    7.131223617812143,
    7.323895587840543,
    7.5195704746346665,
    7.7182615035334345,
    7.919981813454504,
    8.124744458384042,
    8.332562408825165,
    8.543448553206703,
    8.757415699253682,
    8.974476575321063,
    9.194643831691977,
    9.417930041841839,
    9.644347703669503,
    9.873909240696694,
    10.106627003236781,
    10.342513269534024,
    10.58158024687427,
    10.8238400726681,
    11.069304815507364,
    11.317986476196008,
    11.569896988756009,
    11.825048221409341,
    12.083451977536606,
    12.345119996613247,
    12.610063955123938,
    12.878295467455942,
    13.149826086772048,
    13.42466730586372,
    13.702830557985108,
    13.984327217668513,
    14.269168601521828,
    14.55736596900856,
    14.848930523210871,
    15.143873411576273,
    15.44220572664832,
    15.743938506781891,
    16.04908273684337,
    16.35764934889634,
    16.66964922287304,
    16.985093187232053,
    17.30399201960269,
    17.62635644741625,
    17.95219714852476,
    18.281524751807332,
    18.614349837764564,
    18.95068293910138,
    19.290534541298456,
    19.633915083172692,
    19.98083495742689,
    20.331304511189067,
    20.685334046541502,
    21.042933821039977,
    21.404114048223256,
    21.76888489811322,
    22.137256497705877,
    22.50923893145328,
    22.884842241736916,
    23.264076429332462,
    23.6469514538663,
    24.033477234264016,
    24.42366364919083,
    24.817520537484558,
    25.21505769858089,
    25.61628489293138,
    26.021211842414342,
    26.429848230738664,
    26.842203703840827,
    27.258287870275353,
    27.678110301598522,
    28.10168053274597,
    28.529008062403893,
    28.96010235337422,
    29.39497283293396,
    29.83362889318845,
    30.276079891419332,
    30.722335150426627,
    31.172403958865512,
    31.62629557157785,
    32.08401920991837,
    32.54558406207592,
    33.010999283389665,
    33.4802739966603,
    33.953417292456834,
    34.430438229418264,
    34.911345834551085,
    35.39614910352207,
    35.88485700094671,
    36.37747846067349,
    36.87402238606382,
    37.37449765026789,
    37.87891309649659,
    38.38727753828926,
    38.89959975977785,
    39.41588851594697,
    39.93615253289054,
    40.460400508064545,
    40.98864111053629,
    41.520882981230194,
    42.05713473317016,
    42.597404951718396,
    43.141702194811224,
    43.6900349931913,
    44.24241185063697,
    44.798841244188324,
    45.35933162437017,
    45.92389141541209,
    46.49252901546552,
    47.065252796817916,
    47.64207110610409,
    48.22299226451468,
    48.808024568002054,
    49.3971762874833,
    49.9904556690408,
    50.587870934119984,
    51.189430279724725,
    51.79514187861014,
    52.40501387947288,
    53.0190544071392,
    53.637271562750364,
    54.259673423945976,
    54.88626804504493,
    55.517063457223934,
    56.15206766869424,
    56.79128866487574,
    57.43473440856916,
    58.08241284012621,
    58.734331877617365,
    59.39049941699807,
    60.05092333227251,
    60.715611475655585,
    61.38457167773311,
    62.057811747619894,
    62.7353394731159,
    63.417162620860914,
    64.10328893648692,
    64.79372614476921,
    65.48848194977529,
    66.18756403501224,
    66.89098006357258,
    67.59873767827808,
    68.31084450182222,
    69.02730813691093,
    69.74813616640164,
    70.47333615344107,
    71.20291564160104,
    71.93688215501312,
    72.67524319850172,
    73.41800625771542,
    74.16517879925733,
    74.9167682708136,
    75.67278210128072,
    76.43322770089146,
    77.1981124613393,
    77.96744375590167,
    78.74122893956174,
    79.51947534912904,
    80.30219030335869,
    81.08938110306934,
    81.88105503125999,
    82.67721935322541,
    83.4778813166706,
    84.28304815182372,
    85.09272707154808,
    85.90692527145302,
    86.72564993000343,
    87.54890820862819,
    88.3767072518277,
    89.2090541872801,
    90.04595612594655,
    90.88742016217518,
    91.73345337380438,
    92.58406282226491,
    93.43925555268066,
    94.29903859396902,
    95.16341895893969,
    96.03240364439274,
    96.9059996312159,
    97.78421388448044,
    98.6670533535366,
    99.55452497210776,
];

/// 将足够小的弧度角规范到 [0, 2π)。
fn sanitize_radians(angle: f64) -> f64 {
    (angle + std::f64::consts::PI * 8.0) % (std::f64::consts::PI * 2.0)
}

/// 反线性化并返回浮点值（不取整、不截断）。
fn true_delinearized(rgb_component: f64) -> f64 {
    let normalized = rgb_component / 100.0;
    let delinearized = if normalized <= 0.0031308 {
        normalized * 12.92
    } else {
        1.055 * normalized.powf(1.0 / 2.4) - 0.055
    };
    delinearized * 255.0
}

/// 单通道的锥体响应色适应。
fn chromatic_adaptation(component: f64) -> f64 {
    let af = component.abs().powf(0.42);
    signum(component) * 400.0 * af / (af + 27.13)
}

/// 线性 RGB 颜色的 CAM16 色相（弧度）。
fn hue_of(linrgb: [f64; 3]) -> f64 {
    let scaled_discount = matrix_multiply(linrgb, SCALED_DISCOUNT_FROM_LINRGB);
    let r_a = chromatic_adaptation(scaled_discount[0]);
    let g_a = chromatic_adaptation(scaled_discount[1]);
    let b_a = chromatic_adaptation(scaled_discount[2]);
    let a = (11.0 * r_a + -12.0 * g_a + b_a) / 11.0;
    let b = (r_a + g_a - 2.0 * b_a) / 9.0;
    b.atan2(a)
}

/// 判断 a -> b -> c 是否按角度升序环绕。
fn are_in_cyclic_order(a: f64, b: f64, c: f64) -> bool {
    let delta_ab = sanitize_radians(b - a);
    let delta_ac = sanitize_radians(c - a);
    delta_ab < delta_ac
}

/// 求解 lerp(source, target, t) = mid 的 t。
fn intercept(source: f64, mid: f64, target: f64) -> f64 {
    (mid - source) / (target - source)
}

/// 在两点之间插值。
fn lerp_point(source: [f64; 3], t: f64, target: [f64; 3]) -> [f64; 3] {
    [
        source[0] + (target[0] - source[0]) * t,
        source[1] + (target[1] - source[1]) * t,
        source[2] + (target[2] - source[2]) * t,
    ]
}

/// 将线段与某个坐标平面求交。
fn set_coordinate(source: [f64; 3], coordinate: f64, target: [f64; 3], axis: usize) -> [f64; 3] {
    let t = intercept(source[axis], coordinate, target[axis]);
    lerp_point(source, t, target)
}

/// 坐标是否落在 RGB 立方体内。
fn is_bounded(x: f64) -> bool {
    (0.0..=100.0).contains(&x)
}

/// 计算 y 平面与 RGB 立方体交多边形的第 n 个顶点。
fn nth_vertex(y: f64, n: i32) -> [f64; 3] {
    let k_r = Y_FROM_LINRGB[0];
    let k_g = Y_FROM_LINRGB[1];
    let k_b = Y_FROM_LINRGB[2];
    let coord_a = if n % 4 <= 1 { 0.0 } else { 100.0 };
    let coord_b = if n % 2 == 0 { 0.0 } else { 100.0 };
    if n < 4 {
        let g = coord_a;
        let b = coord_b;
        let r = (y - g * k_g - b * k_b) / k_r;
        if is_bounded(r) {
            [r, g, b]
        } else {
            [-1.0, -1.0, -1.0]
        }
    } else if n < 8 {
        let b = coord_a;
        let r = coord_b;
        let g = (y - r * k_r - b * k_b) / k_g;
        if is_bounded(g) {
            [r, g, b]
        } else {
            [-1.0, -1.0, -1.0]
        }
    } else {
        let r = coord_a;
        let g = coord_b;
        let b = (y - r * k_r - g * k_g) / k_b;
        if is_bounded(b) {
            [r, g, b]
        } else {
            [-1.0, -1.0, -1.0]
        }
    }
}

/// 二分找到包含目标色相的线段两端点。
fn bisect_to_segment(y: f64, target_hue: f64) -> ([f64; 3], [f64; 3]) {
    let mut left = [-1.0, -1.0, -1.0];
    let mut right = left;
    let mut left_hue = 0.0;
    let mut right_hue = 0.0;
    let mut initialized = false;
    let mut uncut = true;
    for n in 0..12 {
        let mid = nth_vertex(y, n);
        if mid[0] < 0.0 {
            continue;
        }
        let mid_hue = hue_of(mid);
        if !initialized {
            left = mid;
            right = mid;
            left_hue = mid_hue;
            right_hue = mid_hue;
            initialized = true;
            continue;
        }
        if uncut || are_in_cyclic_order(left_hue, mid_hue, right_hue) {
            uncut = false;
            if are_in_cyclic_order(left_hue, target_hue, mid_hue) {
                right = mid;
                right_hue = mid_hue;
            } else {
                left = mid;
                left_hue = mid_hue;
            }
        }
    }
    (left, right)
}

/// 取两点中点。
fn midpoint(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        (a[0] + b[0]) / 2.0,
        (a[1] + b[1]) / 2.0,
        (a[2] + b[2]) / 2.0,
    ]
}

/// 小于等于 x 的最近临界平面序号。
fn critical_plane_below(x: f64) -> i32 {
    (x - 0.5).floor() as i32
}

/// 大于等于 x 的最近临界平面序号。
fn critical_plane_above(x: f64) -> i32 {
    (x - 0.5).ceil() as i32
}

/// 在给定 Y 与色相下，二分逼近 RGB 立方体边界上的颜色。
fn bisect_to_limit(y: f64, target_hue: f64) -> [f64; 3] {
    let (mut left, mut right) = bisect_to_segment(y, target_hue);
    let mut left_hue = hue_of(left);
    for axis in 0..3 {
        if left[axis] != right[axis] {
            let (mut l_plane, mut r_plane);
            if left[axis] < right[axis] {
                l_plane = critical_plane_below(true_delinearized(left[axis]));
                r_plane = critical_plane_above(true_delinearized(right[axis]));
            } else {
                l_plane = critical_plane_above(true_delinearized(left[axis]));
                r_plane = critical_plane_below(true_delinearized(right[axis]));
            }
            for _ in 0..8 {
                if (r_plane - l_plane).abs() <= 1 {
                    break;
                }
                let m_plane = ((l_plane + r_plane) as f64 / 2.0).floor() as i32;
                let mid_plane_coordinate = CRITICAL_PLANES[m_plane as usize];
                let mid = set_coordinate(left, mid_plane_coordinate, right, axis);
                let mid_hue = hue_of(mid);
                if are_in_cyclic_order(left_hue, target_hue, mid_hue) {
                    right = mid;
                    r_plane = m_plane;
                } else {
                    left = mid;
                    left_hue = mid_hue;
                    l_plane = m_plane;
                }
            }
        }
    }
    midpoint(left, right)
}

/// 色适应响应的逆变换。
fn inverse_chromatic_adaptation(adapted: f64) -> f64 {
    let adapted_abs = adapted.abs();
    let base = (27.13 * adapted_abs / (400.0 - adapted_abs)).max(0.0);
    signum(adapted) * base.powf(1.0 / 0.42)
}

/// 由色相（弧度）、chroma 与 Y 直接求解 ARGB（对应原版 findResultByJ）。
fn find_result_by_j(hue_radians: f64, chroma: f64, y: f64) -> Option<u32> {
    // j 的初始估计。
    let mut j = y.sqrt() * 11.0;
    let vc = default_viewing_conditions();
    let t_inner_coeff = 1.0 / (1.64 - 0.29f64.powf(vc.n)).powf(0.73);
    let e_hue = 0.25 * ((hue_radians + 2.0).cos() + 3.8);
    let p1 = e_hue * (50000.0 / 13.0) * vc.nc * vc.ncb;
    let h_sin = hue_radians.sin();
    let h_cos = hue_radians.cos();
    for iteration_round in 0..5 {
        let j_normalized = j / 100.0;
        let alpha = if chroma == 0.0 || j == 0.0 {
            0.0
        } else {
            chroma / j_normalized.sqrt()
        };
        let t = (alpha * t_inner_coeff).powf(1.0 / 0.9);
        let ac = vc.aw * j_normalized.powf(1.0 / vc.c / vc.z);
        let p2 = ac / vc.nbb;
        let gamma = 23.0 * (p2 + 0.305) * t / (23.0 * p1 + 11.0 * t * h_cos + 108.0 * t * h_sin);
        let a = gamma * h_cos;
        let b = gamma * h_sin;
        let r_a = (460.0 * p2 + 451.0 * a + 288.0 * b) / 1403.0;
        let g_a = (460.0 * p2 - 891.0 * a - 261.0 * b) / 1403.0;
        let b_a = (460.0 * p2 - 220.0 * a - 6300.0 * b) / 1403.0;
        let r_c_scaled = inverse_chromatic_adaptation(r_a);
        let g_c_scaled = inverse_chromatic_adaptation(g_a);
        let b_c_scaled = inverse_chromatic_adaptation(b_a);
        let linrgb = matrix_multiply(
            [r_c_scaled, g_c_scaled, b_c_scaled],
            LINRGB_FROM_SCALED_DISCOUNT,
        );
        if linrgb[0] < 0.0 || linrgb[1] < 0.0 || linrgb[2] < 0.0 {
            return None;
        }
        let k_r = Y_FROM_LINRGB[0];
        let k_g = Y_FROM_LINRGB[1];
        let k_b = Y_FROM_LINRGB[2];
        let fnj = k_r * linrgb[0] + k_g * linrgb[1] + k_b * linrgb[2];
        if fnj <= 0.0 {
            return None;
        }
        if iteration_round == 4 || (fnj - y).abs() < 0.002 {
            if linrgb[0] > 100.01 || linrgb[1] > 100.01 || linrgb[2] > 100.01 {
                return None;
            }
            return Some(argb_from_linrgb(linrgb));
        }
        // 牛顿法迭代，用 2 * fn(j) / j 近似 fn'(j)。
        j = j - (fnj - y) * j / (2.0 * fnj);
    }
    None
}

/// 求解给定 hue / chroma / L* 的 sRGB 颜色（对应原版 HctSolver.solveToInt）。
fn solve_to_int(hue_degrees: f64, chroma: f64, lstar: f64) -> u32 {
    if chroma < 0.0001 || !(0.0001..=99.9999).contains(&lstar) {
        return argb_from_lstar(lstar);
    }
    let hue_degrees = sanitize_degrees_double(hue_degrees);
    let hue_radians = hue_degrees / 180.0 * std::f64::consts::PI;
    let y = y_from_lstar(lstar);
    if let Some(exact_answer) = find_result_by_j(hue_radians, chroma, y) {
        return exact_answer;
    }
    let linrgb = bisect_to_limit(y, hue_radians);
    argb_from_linrgb(linrgb)
}

/// 计算单色调在给定 hue 下的最大 chroma（对应原版 KeyColor.maxChroma）。
fn max_chroma(hue: f64, tone: i32) -> f64 {
    Hct::from_hct(hue, 200.0, tone as f64).chroma()
}

/// 关键色：从 T50 附近二分搜索首个满足请求 chroma 的 tone（对应原版 KeyColor.create）。
fn create_key_color(hue: f64, requested_chroma: f64) -> u32 {
    let pivot_tone = 50;
    let tone_step_size = 1;
    // 接受略高于请求 chroma 的 epsilon。
    let epsilon = 0.01;
    let mut lower_tone = 0;
    let mut upper_tone = 100;
    while lower_tone < upper_tone {
        let mid_tone = (lower_tone + upper_tone) / 2;
        let is_ascending = max_chroma(hue, mid_tone) < max_chroma(hue, mid_tone + tone_step_size);
        let sufficient_chroma = max_chroma(hue, mid_tone) >= requested_chroma - epsilon;
        if sufficient_chroma {
            if (lower_tone - pivot_tone).abs() < (upper_tone - pivot_tone).abs() {
                upper_tone = mid_tone;
            } else if lower_tone == mid_tone {
                return Hct::from_hct(hue, requested_chroma, lower_tone as f64).to_argb();
            } else {
                lower_tone = mid_tone;
            }
        } else if is_ascending {
            lower_tone = mid_tone + tone_step_size;
        } else {
            upper_tone = mid_tone;
        }
    }
    Hct::from_hct(hue, requested_chroma, lower_tone as f64).to_argb()
}

/// 两色各通道取平均（对应原版 TonalPalette.averageArgb）。
fn average_argb(argb1: u32, argb2: u32) -> u32 {
    let red1 = (argb1 >> 16) & 0xff;
    let green1 = (argb1 >> 8) & 0xff;
    let blue1 = argb1 & 0xff;
    let red2 = (argb2 >> 16) & 0xff;
    let green2 = (argb2 >> 8) & 0xff;
    let blue2 = argb2 & 0xff;
    let red = ((red1 + red2) as f64 / 2.0).round() as u32;
    let green = ((green1 + green2) as f64 / 2.0).round() as u32;
    let blue = ((blue1 + blue2) as f64 / 2.0).round() as u32;
    0xff00_0000 | ((red & 0xff) << 16) | ((green & 0xff) << 8) | (blue & 0xff)
}

// ─────────────────────────────── Hct ───────────────────────────────

/// HCT：CAM16 的 hue / chroma 与 L*a*b* 的 tone（L*）组成的颜色系统。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Hct {
    /// 该 HCT 对应的不透明 ARGB。
    argb: u32,
    /// CAM16 色相，范围 [0, 360)。
    hue: f64,
    /// CAM16 色度，>= 0。
    chroma: f64,
    /// L*a*b* 明度，范围 [0, 100]。
    tone: f64,
}

impl Hct {
    /// 由不透明 ARGB 构造（默认观察条件）。
    pub fn from_argb(argb: u32) -> Self {
        let cam = cam16_from_argb(argb);
        Self {
            argb,
            hue: cam.hue,
            chroma: cam.chroma,
            tone: lstar_from_argb(argb),
        }
    }

    /// 由 8 位 RGB 分量构造。
    pub fn from_rgb(r: u8, g: u8, b: u8) -> Self {
        Self::from_argb(argb_from_rgb(r, g, b))
    }

    /// hue 0..360, chroma >= 0, tone 0..100
    pub fn from_hct(hue: f64, chroma: f64, tone: f64) -> Self {
        Self::from_argb(solve_to_int(hue, chroma, tone))
    }

    /// CAM16 色相（度）。
    pub fn hue(&self) -> f64 {
        self.hue
    }

    /// CAM16 色度。
    pub fn chroma(&self) -> f64 {
        self.chroma
    }

    /// L*a*b* 明度。
    pub fn tone(&self) -> f64 {
        self.tone
    }

    /// 返回不透明 ARGB。
    pub fn to_argb(&self) -> u32 {
        self.argb
    }

    /// 黄色相段判定（对应原版 Hct.isYellow）。
    fn is_yellow(hue: f64) -> bool {
        (105.0..125.0).contains(&hue)
    }
}

// ─────────────────────────────── TonalPalette ───────────────────────────────

/// 色调板：在 hue 与 chroma 固定时，按 tone 取色。
#[derive(Debug, Clone, Copy)]
pub struct TonalPalette {
    /// 固定色相。
    hue: f64,
    /// 固定色度。
    chroma: f64,
    /// 关键色。
    key_color: u32,
}

impl TonalPalette {
    /// 由 hue 与 chroma 构造，并计算关键色。
    pub fn from_hue_and_chroma(hue: f64, chroma: f64) -> Self {
        Self {
            hue,
            chroma,
            key_color: create_key_color(hue, chroma),
        }
    }

    /// 返回该色调板在指定 tone（L*）上的 ARGB
    pub fn tone(&self, tone: f64) -> u32 {
        if tone == 99.0 && Hct::is_yellow(self.hue) {
            average_argb(self.tone(98.0), self.tone(100.0))
        } else {
            Hct::from_hct(self.hue, self.chroma, tone).to_argb()
        }
    }

    /// 关键色 ARGB。
    pub fn key_color(&self) -> u32 {
        self.key_color
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 浮点近似比较（hue / chroma / tone 容差 0.5）。
    fn assert_close(actual: f64, expected: f64) {
        assert!(
            (actual - expected).abs() < 0.5,
            "期望 {expected}，实际 {actual}，差值 {}",
            (actual - expected).abs()
        );
    }

    #[test]
    fn test_argb_from_hex_accepts_with_and_without_hash() {
        assert_eq!(argb_from_hex("#6750a4"), Some(0xff67_50a4));
        assert_eq!(argb_from_hex("6750a4"), Some(0xff67_50a4));
        assert_eq!(argb_from_hex("6750A4"), Some(0xff67_50a4));
        assert_eq!(argb_from_hex("#fff"), Some(0xffff_ffff));
    }

    #[test]
    fn test_argb_from_hex_rejects_invalid_input() {
        assert_eq!(argb_from_hex(""), None);
        assert_eq!(argb_from_hex("#12345"), None);
        assert_eq!(argb_from_hex("gggggg"), None);
        assert_eq!(argb_from_hex("#12345g"), None);
    }

    #[test]
    fn test_hex_from_argb_round_trip() {
        let argb = argb_from_hex("#6750a4").unwrap();
        assert_eq!(argb, 0xff67_50a4);
        assert_eq!(hex_from_argb(argb), "#6750a4");
        assert_eq!(hex_from_argb(argb_from_hex("6750A4").unwrap()), "#6750a4");
    }

    #[test]
    fn test_hct_from_argb_golden_vectors() {
        let cases: &[(&str, f64, f64, f64)] = &[
            ("#6750a4", 298.9810, 47.8565, 40.0832),
            ("#3b82f6", 266.3361, 64.1575, 55.6333),
            ("#111111", 209.4955, 0.6610, 5.0633),
            ("#e8e8e8", 209.4923, 2.7102, 91.9959),
            ("#e11d48", 15.1052, 92.2827, 48.7032),
            ("#16a34a", 150.6318, 64.3481, 58.8439),
            ("#f59e0b", 70.5403, 58.3126, 72.1552),
            ("#0ea5e9", 240.5608, 54.8993, 64.0692),
        ];
        for (hex, hue, chroma, tone) in cases {
            let hct = Hct::from_argb(argb_from_hex(hex).unwrap());
            assert_close(hct.hue(), *hue);
            assert_close(hct.chroma(), *chroma);
            assert_close(hct.tone(), *tone);
        }
    }

    #[test]
    fn test_hct_from_hct_to_argb_golden_vectors() {
        let cases: &[(f64, f64, f64, &str)] = &[
            (298.981, 36.0, 40.0, "#65558f"),
            (298.981, 36.0, 80.0, "#cfbdfe"),
            (298.981, 0.0, 98.0, "#f9f9f9"),
            (298.981, 0.0, 6.0, "#131313"),
            (266.3361, 36.0, 40.0, "#445e91"),
            (266.3361, 36.0, 80.0, "#adc6ff"),
            (15.1052, 36.0, 40.0, "#8f4a4f"),
            (210.0, 0.0, 50.0, "#777777"),
            (0.0, 48.0, 40.0, "#984061"),
            (120.0, 16.0, 30.0, "#45492f"),
            (298.981, 47.8565, 40.0832, "#6750a4"),
        ];
        for (hue, chroma, tone, hex) in cases {
            let argb = Hct::from_hct(*hue, *chroma, *tone).to_argb();
            assert_eq!(hex_from_argb(argb), *hex, "hct({hue}, {chroma}, {tone})");
        }
    }

    #[test]
    fn test_tonal_palette_tone_chromatic_golden_vectors() {
        let palette = TonalPalette::from_hue_and_chroma(298.981, 36.0);
        let cases: &[(f64, &str)] = &[
            (0.0, "#000000"),
            (4.0, "#13003a"),
            (6.0, "#18053f"),
            (10.0, "#201047"),
            (12.0, "#24144b"),
            (17.0, "#2f2056"),
            (20.0, "#36275d"),
            (22.0, "#3a2b62"),
            (24.0, "#3f3067"),
            (30.0, "#4d3d75"),
            (40.0, "#65558f"),
            (50.0, "#7e6ea9"),
            (60.0, "#9887c5"),
            (70.0, "#b3a2e1"),
            (80.0, "#cfbdfe"),
            (87.0, "#e1d3ff"),
            (90.0, "#e9ddff"),
            (92.0, "#eee4ff"),
            (94.0, "#f3eaff"),
            (95.0, "#f6eeff"),
            (96.0, "#f8f1ff"),
            (98.0, "#fdf7ff"),
            (99.0, "#fffbff"),
            (100.0, "#ffffff"),
        ];
        for (tone, hex) in cases {
            assert_eq!(hex_from_argb(palette.tone(*tone)), *hex, "tone {tone}");
        }
    }

    #[test]
    fn test_tonal_palette_tone_gray_golden_vectors() {
        let palette = TonalPalette::from_hue_and_chroma(298.981, 0.0);
        let cases: &[(f64, &str)] = &[
            (0.0, "#000000"),
            (4.0, "#0e0e0e"),
            (6.0, "#131313"),
            (10.0, "#1b1b1b"),
            (12.0, "#1f1f1f"),
            (17.0, "#2a2a2a"),
            (20.0, "#303030"),
            (22.0, "#353535"),
            (24.0, "#393939"),
            (30.0, "#474747"),
            (40.0, "#5e5e5e"),
            (50.0, "#777777"),
            (60.0, "#919191"),
            (70.0, "#ababab"),
            (80.0, "#c6c6c6"),
            (87.0, "#dadada"),
            (90.0, "#e2e2e2"),
            (92.0, "#e8e8e8"),
            (94.0, "#eeeeee"),
            (95.0, "#f1f1f1"),
            (96.0, "#f3f3f3"),
            (98.0, "#f9f9f9"),
            (99.0, "#fcfcfc"),
            (100.0, "#ffffff"),
        ];
        for (tone, hex) in cases {
            assert_eq!(hex_from_argb(palette.tone(*tone)), *hex, "tone {tone}");
        }
    }

    #[test]
    fn test_tonal_palette_key_color() {
        let chromatic = TonalPalette::from_hue_and_chroma(298.981, 36.0);
        assert_eq!(hex_from_argb(chromatic.key_color()), "#7e6ea9");
        let gray = TonalPalette::from_hue_and_chroma(298.981, 0.0);
        assert_eq!(hex_from_argb(gray.key_color()), "#777777");
    }

    #[test]
    fn test_hct_from_rgb_matches_from_argb() {
        let from_rgb = Hct::from_rgb(0x67, 0x50, 0xa4);
        let from_argb = Hct::from_argb(0xff67_50a4);
        assert_eq!(from_rgb.to_argb(), from_argb.to_argb());
        assert_close(from_rgb.hue(), from_argb.hue());
        assert_close(from_rgb.chroma(), from_argb.chroma());
        assert_close(from_rgb.tone(), from_argb.tone());
    }
}
