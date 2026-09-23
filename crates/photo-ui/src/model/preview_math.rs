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

/// 框选拖拽 → 图片归一化 bbox（0-1，左上/右下规范化）。
///
/// **两个坐标必须锚在同一个元素上**：
/// - `start_window` / `end_window`：GPUI 鼠标事件的 `position`，是**窗口**坐标
///   （gpui 原文：“The position of the mouse on the window”）；
/// - `image_rect`：**图片显示 div 在窗口里的 bounds** `(x, y, w, h)`，由该 div 自己的
///   `on_prepaint` 实测；叠加框就是这个 div 的绝对定位子节点 → 同坐标系。
///
/// 这样写就不需要知道「视口在窗口里的位置」「图片在视口里的居中/平移偏移」这些中间量
/// （踩过的坑：直接把鼠标的窗口坐标当视口坐标用，框会整体偏移「左活动栏 + 左停靠区 +
/// 顶栏」两三百像素——用 `preview_viewport_origin` 补一层也能对，但多一个可能错的假设；
/// 锚在同一个元素上则由构造保证对齐）。
///
/// 超出显示区的部分夹到 0-1（框选可以拖到图外，不应产生越界 bbox）。
/// `image_rect` 宽或高 <= 0（图片未就绪/未布局）时返回 None。
pub fn region_bbox_from_drag(
    start_window: Vec2,
    end_window: Vec2,
    image_rect: (f64, f64, f64, f64),
) -> Option<photo_domain::BBox> {
    let (origin_x, origin_y, disp_w, disp_h) = image_rect;
    if disp_w <= 0.0 || disp_h <= 0.0 {
        return None;
    }
    let norm = |v: f64, off: f64, d: f64| ((v - off) / d).clamp(0.0, 1.0) as f32;
    let (x1, x2) = {
        let (a, b) = (
            norm(start_window.0, origin_x, disp_w),
            norm(end_window.0, origin_x, disp_w),
        );
        if a <= b { (a, b) } else { (b, a) }
    };
    let (y1, y2) = {
        let (a, b) = (
            norm(start_window.1, origin_y, disp_h),
            norm(end_window.1, origin_y, disp_h),
        );
        if a <= b { (a, b) } else { (b, a) }
    };
    Some(photo_domain::BBox::new(x1, y1, x2, y2))
}
