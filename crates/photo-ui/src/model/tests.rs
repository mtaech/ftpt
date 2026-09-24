//! Appendix B 纯函数单元测试套件。

use chrono::NaiveDate;
use photo_domain::{
    CaptureMeta, ColorLabel, ImageFormat, Rating, RecognitionFilter, RecognitionStatus, SortBy,
    SubjectSummary,
};
use photo_engine::template::{NameTemplateContext, render_name_template};

use super::*;

fn make_meta(base_name: &str, primary_path: &str, primary_format: &str) -> CaptureMeta {
    CaptureMeta {
        index: 0,
        base_name: base_name.to_string(),
        primary_path: primary_path.to_string(),
        primary_format: primary_format.to_string(),
        file_size: Some(1024),
        date_taken: None,
        extensions: vec![],
        camera_make: None,
        camera_model: None,
        lens: None,
        exposure_time: None,
        f_number: None,
        iso: None,
        focal_length: None,
        image_width: None,
        image_height: None,
        gps_lat: None,
        gps_lon: None,
        focus_point: None,
        rating: Rating::None,
        color_label: ColorLabel::None,
        flag: None,
        keywords: vec![],
        taxon_name: None,
        taxon_confidence: None,
        taxon_gap: None,
        recognition_status: None,
        taxon_bbox: None,
        subjects: vec![],
        failure_stage: None,
        candidates: Vec::new(),
    }
}

#[test]
fn test_lens_options_dedup_sorted_and_skips_blank() {
    let mut a = make_meta("a", "/p/a.jpg", "JPEG");
    a.lens = Some("RF24-70mm F2.8".to_string());
    let mut b = make_meta("b", "/p/b.jpg", "JPEG");
    b.lens = Some(" RF24-70mm F2.8 ".to_string()); // 前后空白归一后与 a 相同
    let mut c = make_meta("c", "/p/c.jpg", "JPEG");
    c.lens = Some("".to_string()); // 空串不进候选
    let d = make_meta("d", "/p/d.jpg", "JPEG"); // None 不进候选
    let mut e = make_meta("e", "/p/e.jpg", "JPEG");
    e.lens = Some("EF100-400mm".to_string());

    assert_eq!(
        lens_options(&[a, b, c, d, e]),
        vec!["EF100-400mm".to_string(), "RF24-70mm F2.8".to_string()]
    );
}

#[test]
fn test_taxon_options_include_subjects_and_skip_unrecognized() {
    let mut a = make_meta("a", "/p/a.jpg", "JPEG");
    a.taxon_name = Some("大山雀".to_string());
    a.subjects = vec![
        photo_domain::SubjectSummary {
            display_name: "大山雀".to_string(),
            confidence: Some(70.0),
        },
        photo_domain::SubjectSummary {
            display_name: "远东山雀".to_string(),
            confidence: Some(55.0),
        },
    ];
    let mut b = make_meta("b", "/p/b.jpg", "JPEG");
    // 主体识别失败时占位名不进候选（选了也永远筛不到东西）
    b.subjects = vec![photo_domain::SubjectSummary {
        display_name: "<未识别>".to_string(),
        confidence: None,
    }];

    // 去重 + 按码位排序：大 (e5…) < 远 (e8…)
    assert_eq!(
        taxon_options(&[a, b]),
        vec!["大山雀".to_string(), "远东山雀".to_string()]
    );
}

#[test]
fn test_filter_captures_date_taken_null_excluded_when_range_set() {
    let mut criteria = FilterCriteria::default();
    criteria.date_from = Some(NaiveDate::from_ymd_opt(2026, 1, 1).unwrap());

    let m1 = make_meta("a", "/photos/a.jpg", "JPEG"); // date_taken is None
    let indices = filter_captures(&[m1], &criteria);
    assert!(indices.is_empty(), "dateTaken=null 应被排除");
}

#[test]
fn test_filter_captures_date_taken_unparseable_kept() {
    let mut criteria = FilterCriteria::default();
    criteria.date_from = Some(NaiveDate::from_ymd_opt(2026, 1, 1).unwrap());

    let mut m1 = make_meta("a", "/photos/a.jpg", "JPEG");
    m1.date_taken = Some("not-a-valid-date".to_string());

    let indices = filter_captures(&[m1], &criteria);
    assert_eq!(indices, vec![0], "dateTaken 解析失败应保留");
}

#[test]
fn test_filter_captures_taxon_any_subject_match() {
    // 主主体是乌鸫，但筛选「大山雀」——多主体任一命中即保留
    let mut m = make_meta("a", "/photos/a.jpg", "JPEG");
    m.taxon_name = Some("乌鸫".to_string());
    m.subjects = vec![
        SubjectSummary { display_name: "乌鸫".to_string(), confidence: Some(88.0) },
        SubjectSummary { display_name: "大山雀".to_string(), confidence: Some(77.0) },
    ];
    let mut criteria = FilterCriteria::default();
    criteria.taxon_names = vec!["大山雀".to_string()];
    assert_eq!(filter_captures(&[m.clone()], &criteria), vec![0]);

    // 未命中的物种 → 排除
    criteria.taxon_names = vec!["翠鸟".to_string()];
    assert!(filter_captures(&[m], &criteria).is_empty());
}

#[test]
fn test_filter_captures_taxon_single_subject_fallback() {
    // 旧数据（subjects 空）：退化到主主体 taxon_name 匹配
    let mut m = make_meta("a", "/photos/a.jpg", "JPEG");
    m.taxon_name = Some("乌鸫".to_string());
    let mut criteria = FilterCriteria::default();
    criteria.taxon_names = vec!["乌鸫".to_string()];
    assert_eq!(filter_captures(&[m], &criteria), vec![0]);
}

#[test]
fn test_filter_captures_raw_wildcard() {
    let mut criteria = FilterCriteria::default();
    criteria.format_filter = Some(ImageFormat::Raw("RAW".to_string()));

    let m1 = make_meta("a", "/photos/a.nef", "NEF");
    let m2 = make_meta("b", "/photos/b.jpg", "JPEG");
    let indices = filter_captures(&[m1, m2], &criteria);
    assert_eq!(indices, vec![0], "RAW 通配应匹配 NEF，不匹配 JPEG");
}

#[test]
fn test_filter_captures_min_rating() {
    let mut criteria = FilterCriteria::default();
    criteria.min_rating = Some(Rating::Three);

    let mut m1 = make_meta("a", "/photos/a.jpg", "JPEG");
    m1.rating = Rating::None;
    let mut m2 = make_meta("b", "/photos/b.jpg", "JPEG");
    m2.rating = Rating::Two;
    let mut m3 = make_meta("c", "/photos/c.jpg", "JPEG");
    m3.rating = Rating::Three;
    let mut m4 = make_meta("d", "/photos/d.jpg", "JPEG");
    m4.rating = Rating::Five;

    let indices = filter_captures(&[m1, m2, m3, m4], &criteria);
    assert_eq!(indices, vec![2, 3], "minRating=Three 应排除 None/1/2 星");
}

#[test]
fn test_filter_captures_recognition_filter_not_recognized() {
    let mut criteria = FilterCriteria::default();
    criteria.recognition_filter = RecognitionFilter::NotRecognized;

    let mut m1 = make_meta("a", "/photos/a.jpg", "JPEG");
    m1.recognition_status = None;
    let mut m2 = make_meta("b", "/photos/b.jpg", "JPEG");
    m2.recognition_status = Some(RecognitionStatus::Confirmed);
    let mut m3 = make_meta("c", "/photos/c.jpg", "JPEG");
    m3.recognition_status = Some(RecognitionStatus::Unrecognized);

    let indices = filter_captures(&[m1, m2, m3], &criteria);
    assert_eq!(
        indices,
        vec![0],
        "NotRecognized 只留 recognition_status == None"
    );
}

#[test]
fn test_has_active_filters() {
    let default_c = FilterCriteria::default();
    assert!(!has_active_filters(&default_c), "全默认应为 false");

    let mut c1 = FilterCriteria::default();
    c1.min_rating = Some(Rating::One);
    assert!(has_active_filters(&c1));

    let mut c2 = FilterCriteria::default();
    c2.unflagged_filter = true;
    assert!(has_active_filters(&c2));
}

#[test]
fn test_group_stacks_cross_directory_and_scattered() {
    let m1 = make_meta("DSC_001", "/dir1/DSC_001.jpg", "JPEG");
    let m2 = make_meta("DSC_002", "/dir1/DSC_002.jpg", "JPEG");
    let m3 = make_meta("DSC_001", "/dir2/DSC_001.jpg", "JPEG");
    let m4 = make_meta("DSC_001", "/dir1/DSC_001.nef", "NEF");

    let items = vec![m1, m2, m3, m4];
    let groups = group_stacks(&[0, 1, 2, 3], &items);

    // /dir1 的 DSC_001 应聚合成一组 (含 0 和 3)
    // /dir2 的 DSC_001 应为独立组 (含 2)
    assert_eq!(groups.len(), 3);
    let g_dir1_001 = groups.iter().find(|g| g.members.contains(&0)).unwrap();
    assert_eq!(g_dir1_001.members, vec![0, 3]);
    assert_eq!(g_dir1_001.active, 0); // jpg 优先于 nef
    assert!(g_dir1_001.key.contains("DSC_001"));
}

#[test]
fn test_group_by_time_gap_boundaries() {
    let mut m1 = make_meta("1", "/1.jpg", "JPEG");
    m1.date_taken = Some("2026-01-01T12:00:00".to_string());
    let mut m2 = make_meta("2", "/2.jpg", "JPEG");
    m2.date_taken = Some("2026-01-01T12:00:02".to_string()); // +2000ms
    let mut m3 = make_meta("3", "/3.jpg", "JPEG");
    m3.date_taken = Some("2026-01-01T12:00:05".to_string()); // +3000ms
    let m4 = make_meta("4", "/4.jpg", "JPEG"); // no date

    let items = vec![m1, m2, m3, m4];
    let groups = group_by_time(&[0, 1, 2, 3], &items, STACK_TIME_GAP_MS);

    // 0 和 1 相差 2000ms -> 同组
    // 2 与 1 相差 3000ms -> 新组
    // 3 无时间 -> 独立组 x-3
    assert_eq!(groups.len(), 3);
    assert_eq!(groups[0].members, vec![0, 1]);
    assert_eq!(groups[1].members, vec![2]);
    assert_eq!(groups[2].members, vec![3]);
    assert_eq!(groups[2].key, "x-3");
}

#[test]
fn test_pick_primary_jpeg_over_raw() {
    let m1 = make_meta("a", "/a.nef", "NEF");
    let m2 = make_meta("a", "/a.jpg", "JPEG");
    let items = vec![m1, m2];
    assert_eq!(pick_primary(&[0, 1], &items), 1);

    // 纯 RAW 选组内首个
    let m3 = make_meta("b", "/b.cr3", "CR3");
    let m4 = make_meta("b", "/b.nef", "NEF");
    let items2 = vec![m3, m4];
    assert_eq!(pick_primary(&[0, 1], &items2), 0);
}

#[test]
fn test_compute_burst_groups() {
    let mut m1 = make_meta("1", "/1.jpg", "JPEG");
    m1.date_taken = Some("2026:06:28 10:15:30".to_string());
    let mut m2 = make_meta("2", "/2.jpg", "JPEG");
    m2.date_taken = Some("2026:06:28 10:15:31".to_string());
    let mut m3 = make_meta("3", "/3.jpg", "JPEG");
    m3.date_taken = Some("2026:06:28 10:15:40".to_string()); // 独立单张

    let items = vec![m1, m2, m3];
    let map = compute_burst_groups(&items, 2000);
    assert_eq!(map.len(), 2, "只有 size>=2 的组被登记");
    assert_eq!(map.get(&0).unwrap().size, 2);
    assert_eq!(map.get(&1).unwrap().size, 2);
    assert!(!map.contains_key(&2), "单张不登记");

    // 越界时间 (13月) -> 视为 null
    assert!(parse_exif_date(Some("2026:13:28 10:15:30")).is_none());
}

#[test]
fn test_pick_best_frame_deterministic() {
    let mut m1 = make_meta("1", "/photos/b.jpg", "JPEG");
    m1.file_size = Some(5000);

    let mut m2 = make_meta("2", "/photos/a.jpg", "JPEG");
    m2.file_size = Some(5000);

    let items = vec![m1, m2];
    let best = pick_best_frame(&items).unwrap();
    assert_eq!(best, "/photos/a.jpg", "尺寸并列时路径升序决定");
}

/// 预览图源选择：显示尺寸超过母版像素就要换真原图（否则 1:1 只是放大 2560 母版）。
#[test]
fn test_exceeds_master_res_boundary() {
    // 适应窗口（44MP RAW 在 ~850px 视口里 ≈ 812px）不触发
    assert!(!exceeds_master_res((812.0, 541.0), 2560));
    // 正好等于母版长边不触发（1 像素之内不折腾）
    assert!(!exceeds_master_res((2560.0, 1707.0), 2560));
    assert!(exceeds_master_res((2560.5, 1707.0), 2560));
    // 1:1：显示尺寸 = EXIF 自然尺寸
    assert!(exceeds_master_res((8152.0, 5432.0), 2560));
    // 竖幅按长边判断
    assert!(exceeds_master_res((1707.0, 4000.0), 2560));
}

#[test]
fn test_preview_math() {
    assert_eq!(clamp_pan_axis(500.0, 600.0, 10.0), 0.0); // disp <= container -> 0
    assert_eq!(fit_scale(800.0, 600.0, 400.0, 300.0), 1.0); // 小图不放大

    // 缩放后光标下的点保持不动
    let old_disp = (800.0, 600.0);
    let new_disp = (1000.0, 750.0);
    let container = (1200.0, 900.0);
    let pan = (50.0, -30.0);
    let cursor = (600.0, 450.0);
    let new_pan = pan_after_cursor_zoom(old_disp, new_disp, container, pan, cursor);

    // 检验缩放前后的光标图像相对点
    let old_origin_x = preview_center_offset(old_disp.0, container.0) + pan.0;
    let old_img_pt_x = (cursor.0 - old_origin_x) / old_disp.0;

    let new_origin_x = preview_center_offset(new_disp.0, container.0) + new_pan.0;
    let new_img_pt_x = (cursor.0 - new_origin_x) / new_disp.0;

    assert!((old_img_pt_x - new_img_pt_x).abs() < 1e-6);
}

#[test]
fn test_region_bbox_from_drag_normalizes_and_clamps() {
    // 图片显示 div 在窗口里 (100,50) 起、800x600；拖 (300,200)→(500,350)
    let b = region_bbox_from_drag(
        (300.0, 200.0),
        (500.0, 350.0),
        (100.0, 50.0, 800.0, 600.0),
    )
    .unwrap();
    assert!((b.x1 - 0.25).abs() < 1e-6);
    assert!((b.y1 - 0.25).abs() < 1e-6);
    assert!((b.x2 - 0.5).abs() < 1e-6);
    assert!((b.y2 - 0.5).abs() < 1e-6);

    // 反向拖拽（终点在起点左上）：仍然规范化成左上/右下
    let b = region_bbox_from_drag(
        (500.0, 350.0),
        (300.0, 200.0),
        (100.0, 50.0, 800.0, 600.0),
    )
    .unwrap();
    assert!(b.x1 <= b.x2 && b.y1 <= b.y2);
    assert!((b.x1 - 0.25).abs() < 1e-6 && (b.y2 - 0.5).abs() < 1e-6);

    // 拖出图片显示区：夹到 0-1
    let b = region_bbox_from_drag(
        (-200.0, -100.0),
        (2000.0, 1500.0),
        (100.0, 50.0, 800.0, 600.0),
    )
    .unwrap();
    assert_eq!((b.x1, b.y1, b.x2, b.y2), (0.0, 0.0, 1.0, 1.0));

    // 图片未布局（宽高为 0）：None
    assert!(region_bbox_from_drag((0.0, 0.0), (10.0, 10.0), (0.0, 0.0, 0.0, 0.0)).is_none());
}

/// 回归（用户报「框选时鼠标位置和框的位置不在一个地方」）：GPUI 鼠标事件给的是**窗口**
/// 坐标，而叠加框画在图片显示 div 的坐标系里。两者必须锚在同一个元素上——这里的
/// `image_rect` 就是那个 div 在窗口里的 bounds（由它自己的 `on_prepaint` 实测），
/// 里面已经包含了视口在窗口中的原点（约 313,80：左活动栏 48 + 左停靠区 265、标题栏 36
/// + 顶栏 44）与图片居中/平移偏移。换任何窗口尺寸、侧栏宽度、缩放平移，框都跟着走。
#[test]
fn test_region_bbox_from_drag_anchors_to_image_rect_in_window_space() {
    // 图片 div 落在窗口 (413,130)，尺寸 800x600（= 视口原点 313,80 + 视口内偏移 100,50）
    let image_rect = (413.0, 130.0, 800.0, 600.0);

    // 拖拽起点恰好是图片左上角 → bbox 从 0,0 开始；终点 (613,280) → +200,+150 = 25%
    let b = region_bbox_from_drag((413.0, 130.0), (613.0, 280.0), image_rect).unwrap();
    assert!((b.x1 - 0.0).abs() < 1e-6 && (b.y1 - 0.0).abs() < 1e-6);
    assert!((b.x2 - 0.25).abs() < 1e-6 && (b.y2 - 0.25).abs() < 1e-6);

    // 侧栏/窗口变化 → image_rect 变了，同样的物理拖拽（窗口坐标 +Δ）结果不变：
    // 原点是绝对窗口坐标，偏移多少跟着挪多少
    let shifted = region_bbox_from_drag((913.0, 530.0), (1113.0, 680.0), (913.0, 530.0, 800.0, 600.0))
        .unwrap();
    assert_eq!(
        (b.x1, b.y1, b.x2, b.y2),
        (shifted.x1, shifted.y1, shifted.x2, shifted.y2),
    );

    // 反例（旧 bug 口径）：把窗口坐标直接当「图片内坐标」用，框会整体偏到右下——
    // 明确钉住「不能那样」
    let wrong = region_bbox_from_drag((413.0, 130.0), (613.0, 280.0), (0.0, 0.0, 800.0, 600.0))
        .unwrap();
    assert!(
        (wrong.x1 - b.x1).abs() > 0.4,
        "旧口径应当明显偏移（x1 {} vs {}）",
        wrong.x1,
        b.x1
    );
}

/// Shift 快捷键（用户要求）：按住 Shift + 左键拖拽 = 画框，不必先开工具条 toggle。
#[test]
fn test_starts_region_drag_accepts_toggle_or_shift() {
    // toggle 开 / 按住 Shift，任一即进入画框；两者都没有才是平移
    assert!(starts_region_drag(true, false));
    assert!(starts_region_drag(false, true));
    assert!(starts_region_drag(true, true));
    assert!(!starts_region_drag(false, false));
}

#[test]
fn test_region_hint_text_mentions_shift_and_active_mode() {
    // 常驻态必须把 Shift 快捷键写出来（「在界面上用文字提示」这条需求本身就是它）
    let idle = region_hint_text(false, false);
    assert!(idle.contains("Shift"), "常驻提示要提到 Shift：{idle}");
    assert!(idle.contains("拖拽"), "常驻提示要说明操作方式：{idle}");

    // 按住 Shift 与常驻是两种文案：用户要能看出「现在按着 Shift」
    let held = region_hint_text(false, true);
    assert!(held.contains("Shift") && held != idle, "按住 Shift 应有独立文案：{held}");

    // 显式框选模式：说明松开即识别
    let active = region_hint_text(true, false);
    assert!(active.contains("框选识别已开启"), "开启态文案：{active}");
    assert!(active.contains("松开"), "开启态要说明松开即识别：{active}");

    // 模式优先于 Shift 态（开着 toggle 时按 Shift 不改文案）
    assert_eq!(region_hint_text(true, true), active);
}

#[test]
fn test_preview_dynamic_fit_and_centering() {
    // 模拟用户截图场景：原图 7232 x 5424 (4:3)
    let natural_w = 7232.0;
    let natural_h = 5424.0;
    let pad = 16.0;

    // 视口容器 1: 宽 1400, 高 900
    let container_w = 1400.0;
    let container_h = 900.0;
    let inner_w = container_w - pad * 2.0; // 1368.0
    let inner_h = container_h - pad * 2.0; // 868.0

    let fit = fit_scale(inner_w, inner_h, natural_w, natural_h);
    let disp_w = natural_w * fit;
    let disp_h = natural_h * fit;

    let offset_x = preview_center_offset(disp_w, container_w);
    let offset_y = preview_center_offset(disp_h, container_h);

    // 图片应该严格居中：左右留白相等，上下留白相等
    let left_margin = offset_x;
    let right_margin = container_w - (offset_x + disp_w);
    assert!((left_margin - right_margin).abs() < 1e-6, "水平方向必须严格居中");

    let top_margin = offset_y;
    let bottom_margin = container_h - (offset_y + disp_h);
    assert!((top_margin - bottom_margin).abs() < 1e-6, "垂直方向必须严格居中");

    // 视口容器 2: 窗口最大化或边栏折叠后动态变大到宽 1750, 高 1100
    let new_container_w = 1750.0;
    let new_container_h = 1100.0;
    let new_inner_w = new_container_w - pad * 2.0;
    let new_inner_h = new_container_h - pad * 2.0;

    let new_fit = fit_scale(new_inner_w, new_inner_h, natural_w, natural_h);
    assert!(new_fit > fit, "容器变大后 fit 比例必须动态增大");

    let new_disp_w = natural_w * new_fit;
    let new_disp_h = natural_h * new_fit;
    let new_offset_x = preview_center_offset(new_disp_w, new_container_w);
    let new_offset_y = preview_center_offset(new_disp_h, new_container_h);

    let new_left_margin = new_offset_x;
    let new_right_margin = new_container_w - (new_offset_x + new_disp_w);
    assert!((new_left_margin - new_right_margin).abs() < 1e-6, "变大后水平方向必须仍然严格居中");

    let new_top_margin = new_offset_y;
    let new_bottom_margin = new_container_h - (new_offset_y + new_disp_h);
    assert!((new_top_margin - new_bottom_margin).abs() < 1e-6, "变大后垂直方向必须仍然严格居中");
}

#[test]
fn test_render_name_template() {
    let ctx = NameTemplateContext {
        name: "IMG_001".to_string(),
        species: Some("红嘴相思鸟".to_string()),
        date: Some("2026:05:01 12:00:00".to_string()),
        camera: Some("Nikon Z9".to_string()),
        seq: 7,
    };
    let rendered = render_name_template("{seq}_{species}_{unknown}", &ctx);
    assert_eq!(rendered, "007_红嘴相思鸟_{unknown}");

    // 清洗与兜底
    let rendered_clean = render_name_template("{name} <bad> / \\", &ctx);
    assert_eq!(rendered_clean, "IMG_001 bad");
}
// ── 筛选栏下拉（排序方式 / 列数）──

#[test]
fn test_sort_option_value_round_trip() {
    for sort in [
        SortBy::FileName,
        SortBy::DateTaken,
        SortBy::FileSize,
        SortBy::Rating,
        SortBy::Modified,
    ] {
        assert_eq!(sort_by_from_value(sort_by_value(sort)), sort);
    }
    // 选项表本身也要能和枚举对上（漏一个就会在下拉里少一项）
    assert_eq!(SORT_OPTIONS.len(), 5);
    for (value, _) in SORT_OPTIONS {
        assert_eq!(sort_by_value(sort_by_from_value(value)), value);
    }
}

#[test]
fn test_sort_by_from_value_falls_back_to_file_name() {
    assert_eq!(sort_by_from_value("nope"), SortBy::FileName);
    assert_eq!(sort_by_from_value(""), SortBy::FileName);
}

#[test]
fn test_sort_options_labels_are_unique_and_non_empty() {
    let mut values: Vec<&str> = SORT_OPTIONS.iter().map(|(v, _)| *v).collect();
    values.sort_unstable();
    values.dedup();
    assert_eq!(values.len(), SORT_OPTIONS.len(), "排序 value 有重复");

    let mut labels: Vec<&str> = SORT_OPTIONS.iter().map(|(_, l)| *l).collect();
    labels.sort_unstable();
    labels.dedup();
    assert_eq!(labels.len(), SORT_OPTIONS.len(), "排序 label 有重复");
    assert!(labels.iter().all(|l| !l.is_empty()));
}

#[test]
fn test_grid_columns_from_value_clamps_and_falls_back() {
    assert_eq!(grid_columns_from_value("2"), 2);
    assert_eq!(grid_columns_from_value("5"), 5);
    assert_eq!(grid_columns_from_value("9"), 5);
    assert_eq!(grid_columns_from_value("1"), 2);
    assert_eq!(grid_columns_from_value("abc"), 4);
    assert_eq!(grid_columns_from_value(""), 4);
    // 选项表覆盖 2–5 全部取值
    assert_eq!(GRID_COL_OPTIONS.len(), 4);
    for (value, label) in GRID_COL_OPTIONS {
        let cols = grid_columns_from_value(value);
        assert_eq!(cols.to_string(), value);
        assert_eq!(label, format!("{cols} 列"));
    }
}
