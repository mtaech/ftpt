#![allow(dead_code)]
use gpui::*;
use gpui_component::*;
use gpui_component::scroll::ScrollableElement;
use std::path::PathBuf;
use std::sync::mpsc;
use photo_tool_core::domain::{CaptureMeta, SortBy, SortDirection};
use photo_tool_core::{ops, scanner};

mod texture_manager;
use texture_manager::TextureManager;

struct Theme {
    bg: Rgba, surface: Rgba, surface_alt: Rgba,
    border: Rgba, text: Rgba, text_sec: Rgba, text_dim: Rgba,
    accent: Rgba, accent_dim: Rgba, danger: Rgba, grid_bg: Rgba,
}

impl Theme {
    fn light() -> Self { Self {
        bg: rgb(0xf8fafc), surface: rgb(0xffffff), surface_alt: rgb(0xf1f5f9),
        border: rgb(0xe2e8f0), text: rgb(0x1e293b), text_sec: rgb(0x64748b), text_dim: rgb(0x94a3b8),
        accent: rgb(0x2563eb), accent_dim: rgb(0xbfdbfe), danger: rgb(0xef4444), grid_bg: rgb(0xf1f5f9),
    }}
    fn dark() -> Self { Self {
        bg: rgb(0x0a0a0a), surface: rgb(0x1a1a1a), surface_alt: rgb(0x222222),
        border: rgb(0x2a2a2a), text: rgb(0xe0e0e0), text_sec: rgb(0x888888), text_dim: rgb(0x666666),
        accent: rgb(0x3b82f6), accent_dim: rgb(0x1e3a5f), danger: rgb(0xf87171), grid_bg: rgb(0x101010),
    }}
}

enum ScanEvent { Progress(u32), Complete(Vec<CaptureMeta>), Error }

#[derive(Clone)]
struct DirNode { path: PathBuf, name: String, expanded: bool, children: Vec<DirNode> }
impl DirNode {
    fn new(path: PathBuf) -> Self {
        let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| path.to_string_lossy().to_string());
        let children = if path.exists() {
            std::fs::read_dir(&path).ok().map(|e| e.filter_map(|e| e.ok())
                .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
                .map(|e| DirNode { path: e.path(), name: e.file_name().to_string_lossy().to_string(), expanded: false, children: vec![] })
                .collect()).unwrap_or_default()
        } else { vec![] };
        Self { path, name, expanded: false, children }
    }
}
fn build_initial_tree() -> Vec<DirNode> {
    let mut r = Vec::new();
    if let Some(h) = dirs::home_dir() { let p = h.join("Pictures"); if p.exists() { r.push(DirNode::new(p)); } }
    for b in &["C:/", "D:/"] { let p = PathBuf::from(b); if p.exists() { r.push(DirNode::new(p)); } }
    r
}

struct AppState {
    captures: Vec<CaptureMeta>, filtered_indices: Vec<usize>,
    current_path: Option<PathBuf>, selected_indices: Vec<usize>,
    focused_index: usize, sort_by: SortBy, sort_direction: SortDirection,
    search_text: String, is_scanning: bool, scan_progress: Option<u32>,
    scan_rx: Option<mpsc::Receiver<ScanEvent>>, textures: TextureManager,
    thumbnail_size: u32, dark_mode: bool, theme: Theme,
    menu_visible: bool, menu_pos: Point<Pixels>, menu_target: Option<usize>,
    show_settings: bool, search_focused: bool, dir_tree: Vec<DirNode>,
}

impl AppState {
    fn new() -> Self {
        let cd = dirs::cache_dir().unwrap_or_else(|| PathBuf::from(".cache")).join("PT").join("thumbnails");
        Self {
            captures: vec![], filtered_indices: vec![], current_path: None,
            selected_indices: vec![], focused_index: 0,
            sort_by: SortBy::FileName, sort_direction: SortDirection::Ascending,
            search_text: String::new(), is_scanning: false, scan_progress: None,
            scan_rx: None, textures: TextureManager::new(cd),
            thumbnail_size: 220, dark_mode: false, theme: Theme::light(),
            menu_visible: false, menu_pos: point(px(0.), px(0.)), menu_target: None,
            show_settings: false, search_focused: false, dir_tree: build_initial_tree(),
        }
    }
    fn toggle_theme(&mut self) { self.dark_mode = !self.dark_mode; self.theme = if self.dark_mode { Theme::dark() } else { Theme::light() }; }
    fn start_scan(&mut self, path: PathBuf) {
        let (tx, rx) = mpsc::channel();
        self.scan_rx = Some(rx); self.is_scanning = true; self.current_path = Some(path.clone()); self.scan_progress = Some(0);
        let rt = tx.clone();
        let cb: Box<dyn Fn(u32) + Send> = Box::new(move |pct| { let _ = rt.send(ScanEvent::Progress(pct)); });
        std::thread::spawn(move || {
            match scanner::scan_directory(&path, &["xmp".into()], &Default::default(), Some(cb)) {
                Ok(captures) => { let metas: Vec<CaptureMeta> = captures.iter().map(CaptureMeta::from).collect(); let _ = tx.send(ScanEvent::Complete(metas)); }
                Err(_) => { let _ = tx.send(ScanEvent::Error); }
            }
        });
    }
    fn poll_scan(&mut self) {
        let rx = match self.scan_rx.take() { Some(r) => r, None => return };
        let mut keep = true;
        while let Ok(ev) = rx.try_recv() {
            match ev {
                ScanEvent::Progress(p) => self.scan_progress = Some(p),
                ScanEvent::Complete(c) => { self.captures = c; self.selected_indices.clear(); self.focused_index = 0; self.apply_filters(); self.is_scanning = false; self.scan_progress = None; keep = false; }
                ScanEvent::Error => { self.is_scanning = false; self.scan_progress = None; keep = false; }
            }
        }
        if keep { self.scan_rx = Some(rx); }
    }
    fn apply_filters(&mut self) {
        let mut idx: Vec<usize> = (0..self.captures.len()).collect();
        if !self.search_text.is_empty() { let q = self.search_text.to_lowercase(); idx.retain(|&i| self.captures[i].base_name.to_lowercase().contains(&q)); }
        idx.sort_by(|&a, &b| { let ca = &self.captures[a]; let cb = &self.captures[b];
            let cmp = match self.sort_by { SortBy::FileName => ca.base_name.cmp(&cb.base_name), SortBy::FileSize => ca.file_size.unwrap_or(0).cmp(&cb.file_size.unwrap_or(0)), SortBy::DateTaken => ca.date_taken.cmp(&cb.date_taken), };
            if self.sort_direction == SortDirection::Ascending { cmp } else { cmp.reverse() }
        });
        self.filtered_indices = idx;
    }
    fn open_dialog(&mut self, cx: &mut Context<Self>) {
        let mut d = rfd::FileDialog::new();
        if let Some(c) = &self.current_path { d = d.set_directory(c); }
        if let Some(p) = d.pick_folder() { self.start_scan(p); cx.notify(); }
    }
    fn delete_selected(&mut self) {
        let targets: Vec<usize> = std::mem::take(&mut self.selected_indices);
        for &idx in targets.iter().rev() {
            if let Some(cap) = self.captures.get(idx) {
                let dir = self.current_path.clone().unwrap_or_default();
                let fmt = photo_tool_core::domain::ImageFormat::from_extension(&cap.primary_format.to_lowercase()).unwrap_or(photo_tool_core::domain::ImageFormat::Jpeg);
                let capture = photo_tool_core::domain::Capture {
                    base_name: cap.base_name.clone(), directory: dir,
                    source_files: vec![photo_tool_core::domain::SourceFile { path: PathBuf::from(&cap.primary_path), format: fmt, is_sidecar: false, file_size: cap.file_size, }], primary_index: 0,
                };
                let _ = ops::delete_capture(&capture, photo_tool_core::domain::DeleteMode::Trash);
            }
        }
        if let Some(ref p) = self.current_path { self.start_scan(p.clone()); }
    }
    fn toggle_sort(&mut self, cx: &mut Context<Self>) {
        self.sort_by = match self.sort_by { SortBy::FileName => SortBy::DateTaken, SortBy::DateTaken => SortBy::FileSize, SortBy::FileSize => SortBy::FileName, };
        self.apply_filters(); cx.notify();
    }
    fn sort_label(&self) -> &'static str { match self.sort_by { SortBy::FileName => "Name", SortBy::DateTaken => "Date", SortBy::FileSize => "Size" } }
    fn toggle_select_focused(&mut self) {
        if let Some(&ci) = self.filtered_indices.get(self.focused_index) {
            if let Some(p) = self.selected_indices.iter().position(|&i| i == ci) { self.selected_indices.remove(p); }
            else { self.selected_indices.push(ci); }
        }
    }
    fn toggle_expand(&mut self, path: PathBuf) {
        fn toggle_in(n: &mut [DirNode], p: &PathBuf) -> bool {
            for x in n.iter_mut() { if x.path == *p { x.expanded = !x.expanded; return true; } if toggle_in(&mut x.children, p) { return true; } }
            false
        }
        toggle_in(&mut self.dir_tree, &path);
    }
}

impl Render for AppState {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.poll_scan();
        let (surf, bord, txt, txt2, txt3, acc, accd, grid_bg, bg_c, alt) = if self.dark_mode {
            (rgb(0x1a1a1a), rgb(0x2a2a2a), rgb(0xe0e0e0), rgb(0x888888), rgb(0x666666), rgb(0x3b82f6), rgb(0x1e3a5f), rgb(0x101010), rgb(0x0a0a0a), rgb(0x222222))
        } else {
            (rgb(0xffffff), rgb(0xe2e8f0), rgb(0x1e293b), rgb(0x64748b), rgb(0x94a3b8), rgb(0x2563eb), rgb(0xbfdbfe), rgb(0xf1f5f9), rgb(0xf8fafc), rgb(0xf1f5f9))
        };

        if self.is_scanning {
            let p = self.scan_progress.unwrap_or(0);
            return div().size_full().bg(rgb(0x0a0a0a)).flex().flex_col().items_center().justify_center()
                .child(div().text_lg().text_color(rgb(0xcccccc)).child("Scanning\u{2026}"))
                .child(div().mt_3().w(px(280.)).h(px(4.)).bg(rgb(0x2a2a2a)).rounded_full()
                    .child(div().h_full().bg(acc).rounded_full().w(relative(p as f32 / 100.))))
                .child(div().mt_2().text_xs().text_color(rgb(0x888888)).child(format!("{}%", p)))
                .into_any_element();
        }

        div().size_full().bg(bg_c)
            .child(div().size_full().flex_row()
                .child(self.sidebar(surf, bord, txt, txt2, txt3, acc, alt, cx))
                .child(self.center(surf, bord, txt, txt2, txt3, acc, accd, grid_bg, alt, cx))
                .child(self.preview(surf, bord, txt, txt2, txt3, alt)))
            .child(self.context_menu(surf, bord, txt, alt, cx))
            .child(self.settings_dialog(surf, bord, txt, txt2, acc, alt, cx))
            .into_any_element()
    }
}

impl AppState {
    fn sidebar(&mut self, surf: Rgba, bord: Rgba, txt: Rgba, txt2: Rgba, _txt3: Rgba, acc: Rgba, alt: Rgba, cx: &mut Context<Self>) -> impl IntoElement {
        let tree = self.dir_tree.clone();
        div().w(px(240.)).h_full().bg(surf).border_r_1().border_color(bord).flex_col()
            .child(div().h(px(48.)).px_3().flex_row().items_center().border_b_1().border_color(bord)
                .child(div().text_sm().font_weight(FontWeight::BOLD).text_color(txt).child("Browse"))
                .child(div().flex_1())
                .child(div().cursor_pointer().child("+").on_mouse_down(MouseButton::Left, cx.listener(|this, _e, _w, cx| { this.open_dialog(cx); }))))
            .child(div().px_3().py(px(10.)).text_xs().text_color(txt2).child(self.current_path.as_ref().map(|p| p.to_string_lossy().to_string()).unwrap_or_else(|| "No folder".into())))
            .child(div().flex_1().overflow_y_scrollbar().px_2().pb_2().children(self.render_tree(&tree, 0, txt, alt, bord, cx)))
            .child(div().h(px(3.)).bg(acc))
    }

    fn render_tree(&mut self, nodes: &[DirNode], depth: usize, txt: Rgba, alt: Rgba, _bord: Rgba, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let mut items: Vec<AnyElement> = vec![];
        for node in nodes.iter() {
            let has = !node.children.is_empty() || node.path.is_dir();
            let n = node.clone();
            let is_expanded = node.expanded;
            let child_path = n.path.clone();
            let p2 = n.path.clone();
            let children = n.children.clone();
            items.push(div().flex_row().items_center().gap(px(2.)).px(px(6.)).py(px(6.)).rounded_md()
                .hover(|s| s.bg(alt)).cursor_pointer()
                .child(div().w(px(depth as f32 * 16.)).flex_none())
                .child(div().w(px(14.)).text_xs().text_color(txt).child(if has { if n.expanded { "\u{25bc}" } else { "\u{25b6}" } } else { "" }))
                .child(div().text_xs().text_color(txt).child(n.name.clone()))
                .on_mouse_down(MouseButton::Left, cx.listener(move |this, _e, _w, cx| {
                    if !n.children.is_empty() && n.expanded { this.toggle_expand(child_path.clone()); cx.notify(); return; }
                    this.start_scan(p2.clone()); cx.notify();
                })).into_any());
            if is_expanded && has { let sub = self.render_tree(&children, depth + 1, txt, alt, _bord, cx); for s in sub { items.push(s); } }
        }
        items
    }

    fn center(&mut self, surf: Rgba, bord: Rgba, txt: Rgba, txt2: Rgba, _txt3: Rgba, acc: Rgba, accd: Rgba, grid_bg: Rgba, alt: Rgba, cx: &mut Context<Self>) -> impl IntoElement {
        let ss = self.thumbnail_size as f32;
        let gs: Vec<_> = self.filtered_indices.iter().enumerate().map(|(gi, &ci)| {
            let cap = &self.captures[ci];
            let is_f = self.focused_index == gi; let is_s = self.selected_indices.contains(&ci);
            let bg = if is_f || is_s { accd } else { surf };
            let bc = if is_f || is_s { acc } else { bord };
            let tex = self.textures.get_or_load(&cap.primary_path, self.thumbnail_size);
            let nm = cap.base_name.clone(); let ft = cap.primary_format.clone();
            div().w(px(ss)).bg(bg).border_1().border_color(bc).rounded_md().overflow_hidden().hover(|s| s.border_color(acc)).cursor_pointer()
                .on_mouse_down(MouseButton::Left, cx.listener(move |this, _e, _w, cx| { this.focused_index = gi; this.selected_indices = vec![ci]; this.menu_visible = false; cx.notify(); }))
                .on_mouse_down(MouseButton::Right, cx.listener(move |this, ev: &MouseDownEvent, _w, cx| { this.menu_visible = true; this.menu_pos = ev.position; this.menu_target = Some(ci); this.focused_index = gi; this.selected_indices = vec![ci]; cx.notify(); }))
                .child(match tex { Some(src) => img(src).w(px(ss)).h(px(ss)).object_fit(ObjectFit::Cover).into_any(), None => div().w(px(ss)).h(px(ss)).bg(alt).flex().items_center().justify_center().text_xs().text_color(txt2).child(ft).into_any() })
                .child(div().px(px(6.)).py(px(4.)).child(div().text_xs().text_color(txt).truncate().child(nm)))
        }).collect();

        div().flex_1().h_full().flex_col().min_w(px(400.))
            .child(div().h(px(48.)).bg(surf).border_b_1().border_color(bord).flex_row().items_center().px_3().gap(px(6.))
                .child(div().cursor_pointer().child("+").on_mouse_down(MouseButton::Left, cx.listener(|this, _e, _w, cx| { this.open_dialog(cx); })))
                .child(div().cursor_pointer().child(if self.dark_mode { "\u{2600}" } else { "\u{1f319}" }).on_mouse_down(MouseButton::Left, cx.listener(|this, _e, _w, cx| { this.toggle_theme(); cx.notify(); })))
                .child(div().flex_1())
                .child(if self.search_focused {
                    div().h(px(30.)).bg(alt).border_1().border_color(acc).rounded_md().flex_row().items_center().px_2().gap(px(6.))
                        .child(div().text_xs().text_color(txt).child(self.search_text.clone())).into_any()
                } else { div().cursor_pointer().child("search").on_mouse_down(MouseButton::Left, cx.listener(|this, _e, _w, cx| { this.search_focused = true; cx.notify(); })).into_any() })
                .child(div().h(px(20.)).w(px(1.)).bg(bord))
                .child(div().cursor_pointer().child(self.sort_label()).on_mouse_down(MouseButton::Left, cx.listener(|this, _e, _w, cx| { this.toggle_sort(cx); })))
                .child(div().cursor_pointer().child(if self.sort_direction == SortDirection::Ascending { "\u{2191}" } else { "\u{2193}" }).on_mouse_down(MouseButton::Left, cx.listener(|this, _e, _w, cx| { let d = if this.sort_direction == SortDirection::Ascending { SortDirection::Descending } else { SortDirection::Ascending }; this.sort_direction = d; this.apply_filters(); cx.notify(); })))
                .child(div().h(px(20.)).w(px(1.)).bg(bord))
                .child(div().cursor_pointer().child("\u{2699}").on_mouse_down(MouseButton::Left, cx.listener(|this, _e, _w, cx| { this.show_settings = !this.show_settings; cx.notify(); })))
                .child(div().cursor_pointer().child("\u{1f5d1}").on_mouse_down(MouseButton::Left, cx.listener(|this, _e, _w, cx| { this.delete_selected(); cx.notify(); }))))
            .child(div().flex_1().bg(grid_bg).overflow_y_scrollbar()
                .on_key_down(cx.listener(|this, ev: &KeyDownEvent, _, cx| {
                    if this.search_focused {
                        match ev.keystroke.key.as_str() {
                            "escape" => { this.search_focused = false; this.search_text.clear(); this.apply_filters(); cx.notify(); }
                            "backspace" => { this.search_text.pop(); this.apply_filters(); cx.notify(); }
                            "enter" => { this.search_focused = false; cx.notify(); }
                            k if k.len() == 1 => { this.search_text.push_str(k); this.apply_filters(); cx.notify(); }
                            _ => {}
                        } return;
                    }
                    match ev.keystroke.key.as_str() {
                        "right" | "down" => { if this.focused_index + 1 < this.filtered_indices.len() { this.focused_index += 1; cx.notify(); } }
                        "left" | "up" => { this.focused_index = this.focused_index.saturating_sub(1); cx.notify(); }
                        "space" => { this.toggle_select_focused(); cx.notify(); }
                        "delete" => { this.delete_selected(); cx.notify(); }
                        "escape" => { this.menu_visible = false; this.selected_indices.clear(); cx.notify(); }
                        _ => {}
                    }
                }))
                .child(div().p(px(8.)).flex().flex_wrap().gap(px(8.)).children(gs)))
            .child(div().h(px(28.)).bg(surf).border_t_1().border_color(bord).px_3().text_xs().text_color(txt2)
                .flex_row().items_center().child(format!("{} / {}  \u{00b7}  {} selected", self.filtered_indices.len(), self.captures.len(), self.selected_indices.len())))
    }

    fn preview(&mut self, surf: Rgba, bord: Rgba, txt: Rgba, txt2: Rgba, txt3: Rgba, _alt: Rgba) -> impl IntoElement {
        let cap = self.filtered_indices.get(self.focused_index).and_then(|&i| self.captures.get(i));
        let info = cap.map(|c| (c.primary_path.clone(), c.base_name.clone(), c.primary_format.clone(), c.file_size, c.has_xmp, c.stack_count));
        div().w(px(320.)).h_full().bg(surf).border_l_1().border_color(bord).flex_col()
            .child(if let Some((path, name, fmt, fsize, xmp, stack)) = info {
                let tex = self.textures.get_or_load(&path, 600);
                div().flex_1().flex_col()
                    .child(div().flex_1().bg(rgb(0x0a0a0a)).flex().items_center().justify_center()
                        .child(match tex { Some(s) => img(s).object_fit(ObjectFit::Contain).w_full().h_full().into_any(), None => div().text_color(txt3).child("Loading").into_any() }))
                    .child(div().p_3().flex_col().gap(px(4.))
                        .child(div().font_weight(FontWeight::BOLD).text_sm().text_color(txt).child(name))
                        .child(div().text_xs().text_color(txt2).child(format!("{}  \u{00b7}  {} KB", fmt, fsize.map(|s| s / 1024).unwrap_or(0))))
                        .child(div().text_xs().text_color(txt3).child(format!("Stack: {}  \u{00b7}  XMP: {}", stack, if xmp { "Yes" } else { "No" }))))
                    .into_any()
            } else { div().flex_1().flex().items_center().justify_center().text_sm().text_color(txt3).child("Select an image").into_any() })
    }

    fn context_menu(&mut self, surf: Rgba, bord: Rgba, txt: Rgba, alt: Rgba, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.menu_visible { return div().into_any(); }
        let cpath = self.menu_target.and_then(|i| self.captures.get(i)).map(|c| c.primary_path.clone());
        let p2 = cpath.clone();
        div().absolute().left(self.menu_pos.x).top(self.menu_pos.y)
            .bg(surf).border_1().border_color(bord).rounded_lg().shadow_lg().text_sm().flex_col().min_w(px(160.)).py(px(4.))
            .child(div().px_3().py(px(6.)).hover(|s| s.bg(alt)).cursor_pointer()
                .on_mouse_down(MouseButton::Left, cx.listener(|this, _, _, cx| { this.delete_selected(); this.menu_visible = false; cx.notify(); }))
                .text_color(txt).child("Delete"))
            .child(if cpath.is_some() {
                let pp = p2.unwrap();
                div().px_3().py(px(6.)).hover(|s| s.bg(alt)).cursor_pointer()
                    .on_mouse_down(MouseButton::Left, cx.listener(move |this, _, _, cx| { let _ = std::process::Command::new("explorer").arg("/select,").arg(&pp).spawn(); this.menu_visible = false; cx.notify(); }))
                    .text_color(txt).child("Show in Explorer").into_any()
            } else { div().into_any() })
            .into_any()
    }

    fn settings_dialog(&mut self, surf: Rgba, bord: Rgba, txt: Rgba, txt2: Rgba, acc: Rgba, alt: Rgba, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.show_settings { return div().into_any(); }
        let size = self.thumbnail_size;
        div().absolute().inset_0().bg(rgb(0)).opacity(0.4).flex().items_center().justify_center()
            .on_mouse_down(MouseButton::Left, cx.listener(|this, _, _, cx| { this.show_settings = false; cx.notify(); }))
            .child(div().bg(surf).rounded_lg().shadow_2xl().p_5().flex_col().gap(px(12.)).min_w(px(340.)).opacity(1.)
                .on_mouse_down(MouseButton::Left, |_, _, _| {})
                .child(div().text_sm().font_weight(FontWeight::BOLD).text_color(txt).flex_row().justify_between()
                    .child("Settings").child(div().cursor_pointer().child("x").on_mouse_down(MouseButton::Left, cx.listener(|this, _e, _w, cx| { this.show_settings = false; cx.notify(); }))))
                .child(div().h(px(1.)).bg(bord))
                .child(div().text_xs().text_color(txt2).child(format!("Thumbnail Size: {}px", size)))
                .child(div().flex_row().gap(px(6.)).children(vec![120u32, 160, 200, 220, 260, 300, 400].into_iter().map(|s| {
                    let sel = s == size;
                    div().px_3().py(px(6.)).rounded_md().cursor_pointer().bg(if sel { acc } else { alt }).text_color(if sel { rgb(0xffffff) } else { txt }).hover(|s| s.bg(if sel { acc } else { bord })).text_xs().child(s.to_string())
                        .on_mouse_down(MouseButton::Left, cx.listener(move |this, _e, _w, cx| { this.thumbnail_size = s; cx.notify(); }))
                }).collect::<Vec<_>>()))
                .child(div().h(px(1.)).bg(bord))
                .child(div().flex_row().items_center().gap(px(8.))
                    .child(div().text_xs().text_color(txt).child("Theme"))
                    .child(div().flex_1())
                    .child(div().cursor_pointer().child(if self.dark_mode { "Light" } else { "Dark" }).on_mouse_down(MouseButton::Left, cx.listener(|this, _e, _w, cx| { this.toggle_theme(); cx.notify(); })))))
            .into_any()
    }
}

fn main() {
    let app = gpui_platform::application().with_assets(gpui_component_assets::Assets);
    app.run(move |cx| {
        gpui_component::init(cx);
        cx.spawn(async move |cx| {
            cx.open_window(WindowOptions::default(), |window, cx| {
                let view = cx.new(|_cx| AppState::new());
                cx.new(|cx| Root::new(view, window, cx))
            }).expect("open window");
        }).detach();
    });
}
