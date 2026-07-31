// 预览缩放/平移纯函数（与 preview.rs 渲染公式严格一致，改动需同步）。
// 自 state/app.rs 拆出，纯移动，无逻辑改动。

// ── 预览缩放/平移纯函数（与 preview.rs 渲染公式严格一致，改动需同步）──

/// 预览居中偏移：图片 ≤ 容器时居中（≥0），> 容器时为负（图片向左上溢出）
pub(crate) fn preview_center_offset(disp: f32, container: f32) -> f32 {
    (container - disp) / 2.
}

/// 单轴平移钳制：图片 ≤ 容器时不允许平移；> 容器时边缘不可进入视口
pub(crate) fn clamp_pan_axis(disp: f32, container: f32, pan: f32) -> f32 {
    if disp <= container {
        0.
    } else {
        let center = preview_center_offset(disp, container);
        pan.clamp(container - disp - center, -center)
    }
}

/// 光标中心缩放：保持光标下的图像点不动，返回新 pan。
/// old_disp/new_disp 为缩放前后显示尺寸，container 为容器尺寸，cursor 为容器坐标。
pub(crate) fn pan_after_cursor_zoom(
    old_disp: (f32, f32),
    new_disp: (f32, f32),
    container: (f32, f32),
    pan: (f32, f32),
    cursor: (f32, f32),
) -> (f32, f32) {
    let axis = |old_d: f32, new_d: f32, c: f32, p: f32, cur: f32| {
        let old_origin = preview_center_offset(old_d, c) + p;
        let r = if old_d > 0. { new_d / old_d } else { 1. };
        let new_origin = cur - (cur - old_origin) * r;
        new_origin - preview_center_offset(new_d, c)
    };
    (
        axis(old_disp.0, new_disp.0, container.0, pan.0, cursor.0),
        axis(old_disp.1, new_disp.1, container.1, pan.1, cursor.1),
    )
}

/// 窗口坐标 → 图片归一化坐标（0-1，相对原图）。
///
/// 与 preview.rs 渲染公式严格一致（改动需同步）：
/// 图片左上角窗口坐标 = 图片区原点 + p_4 内边距 + 居中偏移 + 平移。
/// 返回值可能出界（超出 [0,1]），由调用方钳制。
pub(crate) fn window_pos_to_image_norm(
    wx: f32,
    wy: f32,
    area: (f32, f32, f32, f32),
    img: (u32, u32),
    zoom: f32,
    pan: (f32, f32),
) -> Option<(f32, f32)> {
    if area.2 <= 0. || img.0 == 0 || img.1 == 0 {
        return None;
    }
    let pad = 16.0;
    let container_w = (area.2 - pad * 2.).max(1.);
    let container_h = (area.3 - pad * 2.).max(1.);
    let scale = (container_w / img.0 as f32)
        .min(container_h / img.1 as f32)
        .min(1.0);
    let (fit_w, fit_h) = (img.0 as f32 * scale, img.1 as f32 * scale);
    let (disp_w, disp_h) = if zoom == 0.0 {
        (img.0 as f32, img.1 as f32)
    } else {
        (fit_w * zoom, fit_h * zoom)
    };
    if disp_w <= 0. || disp_h <= 0. {
        return None;
    }
    let img_left = area.0 + pad + preview_center_offset(disp_w, container_w) + pan.0;
    let img_top = area.1 + pad + preview_center_offset(disp_h, container_h) + pan.1;
    Some(((wx - img_left) / disp_w, (wy - img_top) / disp_h))
}
