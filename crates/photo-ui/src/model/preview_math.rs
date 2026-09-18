//! 预览缩放/平移纯数学函数（对应 §4.6）。

pub type Vec2 = (f64, f64);

/// 预览居中偏移：图片 <= 容器时居中（>=0），> 容器时为负
pub fn preview_center_offset(disp: f64, container: f64) -> f64 {
    (container - disp) / 2.0
}

/// 单轴平移钳制：图片 <= 容器时不允许平移；> 容器时边缘不可进入视口
pub fn clamp_pan_axis(disp: f64, container: f64, pan: f64) -> f64 {
    if disp <= container {
        return 0.0;
    }
    let center = preview_center_offset(disp, container);
    pan.clamp(container - disp - center, -center)
}

/// 光标中心缩放：保持光标下的图像点不动，返回新 pan
pub fn pan_after_cursor_zoom(
    old_disp: Vec2,
    new_disp: Vec2,
    container: Vec2,
    pan: Vec2,
    cursor: Vec2,
) -> Vec2 {
    let axis = |old_d: f64, new_d: f64, c: f64, p: f64, cur: f64| -> f64 {
        let old_origin = preview_center_offset(old_d, c) + p;
        let r = if old_d > 0.0 { new_d / old_d } else { 1.0 };
        let new_origin = cur - (cur - old_origin) * r;
        new_origin - preview_center_offset(new_d, c)
    };

    (
        axis(old_disp.0, new_disp.0, container.0, pan.0, cursor.0),
        axis(old_disp.1, new_disp.1, container.1, pan.1, cursor.1),
    )
}

/// 显示尺寸是否已超过母版可用像素——超过就该换真原图，否则只是在放大母版。
///
/// 母版长边固定为 `master_long`（RAW 走 `MASTER_SIZE`）；1:1 属于此列
/// （显示尺寸 = EXIF 自然尺寸，远超母版），高倍放大同理。
pub fn exceeds_master_res(disp: Vec2, master_long: u32) -> bool {
    disp.0.max(disp.1) > f64::from(master_long)
}

/// 适应缩放系数（fit）：图片完整放入容器，小图不放大（上限 1.0）
pub fn fit_scale(container_w: f64, container_h: f64, img_w: f64, img_h: f64) -> f64 {
    if img_w <= 0.0 || img_h <= 0.0 {
        return 1.0;
    }
    (container_w / img_w).min(container_h / img_h).min(1.0)
}
