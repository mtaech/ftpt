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

enum ScanEvent { Progress(u32), Complete(Vec<CaptureMeta>), Error }

#[derive(Clone)]
struct DirNode { path: PathBuf, name: String, expanded: bool, children: Vec<DirNode> }

impl DirNode {
    fn new(path: PathBuf) -> Self {
        let name = path.file_name().map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| path.to_string_lossy().to_string());
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
    let mut roots = Vec::new();
    if let Some(home) = dirs::home_dir() {
        let pics = home.join("Pictures"); if pics.exists() { roots.push(DirNode::new(pics)); }
    }
    for base in &["C:/", "D:/"] { let p = PathBuf::from(base); if p.exists() { roots.push(DirNode::new(p)); } }
    roots
}

struct AppState {
    captures: Vec<CaptureMeta>, filtered_indices: Vec<usize>,
    current_path: Option<PathBuf>, selected_indices: Vec<usize>,
    focused_index: usize, sort_by: SortBy, sort_direction: SortDirection,
    search_text: String, is_scanning: bool, scan_progress: Option<u32>,
    scan_rx: Option<mpsc::Receiver<ScanEvent>>, textures: TextureManager,
    thumbnail_size: u32, dark_mode: bool,
    menu_visible: bool, menu_pos: Point<Pixels>, menu_target: Option<usize>,
    show_settings: bool, search_focused: bool, dir_tree: Vec<DirNode>,
}

impl AppState {
    fn new() -> Self {
        let cache_dir = dirs::cache_dir().unwrap_or_else(|| PathBuf::from(".cache")).join("PT").join("thumbnails");
        Self {
            captures: vec![], filtered_indices: vec![], current_path: None,
            selected_indices: vec![], focused_index: 0,
            sort_by: SortBy::FileName, sort_direction: SortDirection::Ascending,
            search_text: String::new(), is_scanning: false, scan_progress: None,
            scan_rx: None, textures: TextureManager::new(cache_dir),
            thumbnail_size: 220, dark_mode: false,
            menu_visible: false, menu_pos: point(px(0.), px(0.)), menu_target: None,
            show_settings: false, search_focused: false, dir_tree: build_initial_tree(),
        }
    }

    fn start_scan(&mut self, path: PathBuf) {
        let (tx, rx) = mpsc::channel();
        self.scan_rx = Some(rx); self.is_scanning = true; self.current_path = Some(path.clone()); self.scan_progress = Some(0);
        let report_tx = tx.clone();
        let on_progress: Box<dyn Fn(u32) + Send> = Box::new(move |pct| { let _ = report_tx.send(ScanEvent::Progress(pct)); });
        std::thread::spawn(move || {
            match scanner::scan_directory(&path, &["xmp".into()], &Default::default(), Some(on_progress)) {
                Ok(captures) => { let metas: Vec<CaptureMeta> = captures.iter().map(CaptureMeta::from).collect(); let _ = tx.send(ScanEvent::Complete(metas)); }
                Err(_) => { let _ = tx.send(ScanEvent::Error); }
            }
        });
    }

    fn poll_scan(&mut self) {
        let rx = match self.scan_rx.take() { Some(r) => r, None => return };
        let mut keep = true;
        while let Ok(event) = rx.try_recv() {
            match event {
                ScanEvent::Progress(pct) => self.scan_progress = Some(pct),
                ScanEvent::Complete(captures) => {
                    self.captures = captures; self.selected_indices.clear(); self.focused_index = 0;
                    self.apply_filters(); self.is_scanning = false; self.scan_progress = None; keep = false;
                }
                ScanEvent::Error => { self.is_scanning = false; self.scan_progress = None; keep = false; }
            }
        }
        if keep { self.scan_rx = Some(rx); }
    }

    fn apply_filters(&mut self) {
        let mut indices: Vec<usize> = (0..self.captures.len()).collect();
        if !self.search_text.is_empty() {
            let q = self.search_text.to_lowercase();
            indices.retain(|&i| self.captures[i].base_name.to_lowercase().contains(&q));
        }
        indices.sort_by(|&a, &b| {
            let ca = &self.captures[a]; let cb = &self.captures[b];
            let cmp = match self.sort_by {
                SortBy::FileName => ca.base_name.cmp(&cb.base_name),
                SortBy::FileSize => ca.file_size.unwrap_or(0).cmp(&cb.file_size.unwrap_or(0)),
                SortBy::DateTaken => ca.date_taken.cmp(&cb.date_taken),
            };
            match self.sort_direction { SortDirection::Ascending => cmp, SortDirection::Descending => cmp.reverse() }
        });
        self.filtered_indices = indices;
    }

    fn open_dialog(&mut self, cx: &mut Context<Self>) {
        let mut dialog = rfd::FileDialog::new();
        if let Some(current) = &self.current_path { dialog = dialog.set_directory(current); }
        if let Some(path) = dialog.pick_folder() { self.start_scan(path); cx.notify(); }
    }

    fn delete_selected(&mut self) {
        let targets: Vec<usize> = std::mem::take(&mut self.selected_indices);
        for &idx in targets.iter().rev() {
            if let Some(cap) = self.captures.get(idx) {
                let dir = self.current_path.clone().unwrap_or_default();
                let fmt_str = cap.primary_format.to_lowercase();
                let fmt = photo_tool_core::domain::ImageFormat::from_extension(&fmt_str)
                    .unwrap_or(photo_tool_core::domain::ImageFormat::Jpeg);
                let capture = photo_tool_core::domain::Capture {
                    base_name: cap.base_name.clone(), directory: dir,
                    source_files: vec![photo_tool_core::domain::SourceFile {
                        path: PathBuf::from(&cap.primary_path), format: fmt, is_sidecar: false, file_size: cap.file_size,
                    }], primary_index: 0,
                };
                let _ = ops::delete_capture(&capture, photo_tool_core::domain::DeleteMode::Trash);
            }
        }
        if let Some(ref path) = self.current_path { self.start_scan(path.clone()); }
    }

    fn toggle_sort(&mut self, cx: &mut Context<Self>) {
        self.sort_by = match self.sort_by {
            SortBy::FileName => SortBy::DateTaken, SortBy::DateTaken => SortBy::FileSize, SortBy::FileSize => SortBy::FileName,
        };
        self.apply_filters(); cx.notify();
    }
    fn sort_label(&self) -> &'static str {
        match self.sort_by { SortBy::FileName => "Name", SortBy::DateTaken => "Date", SortBy::FileSize => "Size" }
    }
    fn toggle_select_focused(&mut self) {
        if let Some(&ci) = self.filtered_indices.get(self.focused_index) {
            if let Some(pos) = self.selected_indices.iter().position(|&i| i == ci) { self.selected_indices.remove(pos); }
            else { self.selected_indices.push(ci); }
        }
    }
}

impl Render for AppState {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.poll_scan();
        if self.is_scanning { return self.render_scanning().into_any_element(); }
        let (surf, border) = if self.dark_mode { (rgb(0x252525), rgb(0x3a3a3a)) } else { (rgb(0xffffff), rgb(0xe0e0e0)) };

        div().size_full().bg(if self.dark_mode { rgb(0x1a1a1a) } else { rgb(0xf5f5f5) }).text_color(rgb(0x333333)).flex_row()
            .child(self.sidebar(surf, border, cx))
            .child(self.center(surf, border, cx))
            .child(self.preview(surf))
            .child(self.context_menu(surf, border, cx))
            .child(self.settings_dialog(surf, cx))
            .into_any_element()
    }
}

impl AppState {
    fn render_scanning(&self) -> impl IntoElement {
        let p = self.scan_progress.unwrap_or(0);
        div().size_full().bg(rgb(0x1e1e1e)).flex().flex_col().items_center().justify_center()
            .text_xl().text_color(rgb(0xffffff))
            .child(format!("Scanning: {}%", p))
            .child(div().mt_2().w(px(300.)).h(px(6.)).bg(rgb(0x444444)).rounded_full()
                .child(div().h_full().bg(rgb(0x3b82f6)).rounded_full().w(relative(p as f32 / 100.))))
    }

    fn sidebar(&mut self, surf: Rgba, border: Rgba, cx: &mut Context<Self>) -> impl IntoElement {
        div().w(px(240.)).h_full().bg(surf).border_r_1().border_color(border).overflow_y_scrollbar().p_2()
            .child(div().text_sm().font_weight(FontWeight::BOLD).mb_2().flex_row().items_center().justify_between()
                .child("Directories")
                .child(button::Button::new("open-folder").label("+")
                    .on_click(cx.listener(|this, _, _, cx| { this.open_dialog(cx); }))))
            .child(div().text_xs().text_color(rgb(0x888888)).mb_2()
                .child(self.current_path.as_ref().map(|p| format!("📁 {}", p.display())).unwrap_or_else(|| "No folder".into())))
            .child(div().flex_col().children({
                let tree = self.dir_tree.clone();
                self.render_tree_nodes(&tree, 0, cx)
            }))
    }

    fn toggle_expand(&mut self, path: PathBuf) {
        fn toggle_in(nodes: &mut [DirNode], path: &PathBuf) -> bool {
            for n in nodes.iter_mut() {
                if n.path == *path { n.expanded = !n.expanded; return true; }
                if toggle_in(&mut n.children, path) { return true; }
            }
            false
        }
        toggle_in(&mut self.dir_tree, &path);
    }

    fn render_tree_nodes(&mut self, nodes: &[DirNode], depth: usize, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let mut items: Vec<gpui::AnyElement> = vec![];
        for node in nodes.iter() {
            let has_children = !node.children.is_empty() || node.path.is_dir();
            let n = node.clone();
            let p = node.path.clone();
            let items_ref = &mut items;

            // expander button
            let expander_div = div().w(px(12.)).text_xs().text_color(rgb(0x888888)).cursor_pointer()
                .child(if has_children { if n.expanded { "▼" } else { "▶" } } else { "  " });

            // the row with proper click zones
            let row = div().flex_row().items_center().gap(px(2.))
                .child(div().w(relative(depth as f32 * 16.)).flex_none())
                .child(expander_div.on_mouse_down(MouseButton::Left, cx.listener(move |this, _e, _w, cx| {
                    this.toggle_expand(p.clone()); cx.notify();
                })))
                .child(div().text_xs().px_1().py(px(2.)).rounded_sm().hover(|s| s.bg(rgb(0xe8e8e8))).cursor_pointer()
                    .on_mouse_down(MouseButton::Left, cx.listener(move |this, _e, _w, cx| {
                        this.start_scan(n.path.clone()); cx.notify();
                    }))
                    .child(n.name.clone()));
            items_ref.push(row.into_any());

            if node.expanded && has_children {
                let subs = self.render_tree_nodes(&n.children, depth + 1, cx);
                for s in subs { items_ref.push(s); }
            }
        }
        items
    }

    fn center(&mut self, surf: Rgba, border: Rgba, cx: &mut Context<Self>) -> impl IntoElement {
        let ss = self.thumbnail_size as f32;
        let grid_items: Vec<_> = self.filtered_indices.iter().enumerate().map(|(gi, &ci)| {
            let cap = &self.captures[ci];
            let is_focused = self.focused_index == gi; let is_selected = self.selected_indices.contains(&ci);
            let cell_bg = if is_focused { rgb(0xbfdbfe) } else if is_selected { rgb(0xdbeafe) } else { surf };
            let tex = self.textures.get_or_load(&cap.primary_path, self.thumbnail_size);
            let name = cap.base_name.clone(); let fmt = cap.primary_format.clone(); let xmp = cap.has_xmp;
            div().w(px(ss)).flex_col().bg(cell_bg).rounded_md()
                .border_1().border_color(if is_focused { rgb(0x3b82f6) } else { border }).cursor_pointer()
                .on_mouse_down(MouseButton::Left, cx.listener(move |this, _e, _w, cx| {
                    this.focused_index = gi; this.selected_indices = vec![ci]; this.menu_visible = false; cx.notify();
                }))
                .on_mouse_down(MouseButton::Right, cx.listener(move |this, event: &MouseDownEvent, _w, cx| {
                    this.menu_visible = true; this.menu_pos = event.position;
                    this.menu_target = Some(ci); this.focused_index = gi; this.selected_indices = vec![ci]; cx.notify();
                }))
                .child(match tex {
                    Some(src) => img(src).w(px(ss)).h(px(ss)).rounded_t_md().into_any(),
                    None => div().w(px(ss)).h(px(ss)).bg(rgb(0xe5e5e5)).rounded_t_md()
                        .flex().items_center().justify_center().text_xs().text_color(rgb(0x999999))
                        .child(format!("{} {}", fmt, if xmp { "📝" } else { "" })).into_any(),
                })
                .child(div().px(px(4.)).text_xs().truncate().child(name))
        }).collect();

        div().flex_1().h_full().flex_col()
            .child(div().h(px(36.)).bg(surf).border_b_1().border_color(border)
                .flex_row().items_center().px_2().gap(px(8.))
                .child(format!("{} files", self.captures.len())).child(div().flex_1())
                .child(if self.search_focused {
                    div().text_xs().text_color(rgb(0x2563eb)).child(format!("🔍 `{}`", self.search_text)).into_any()
                } else {
                    button::Button::new("search-btn").label("🔍 search")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.search_focused = true; cx.notify();
                        })).into_any_element()
                })
                .child(button::Button::new("sort-btn").label(self.sort_label())
                    .on_click(cx.listener(|this, _, _, cx| { this.toggle_sort(cx); })))
                .child(button::Button::new("sort-dir")
                    .label(if self.sort_direction == SortDirection::Ascending { "↑" } else { "↓" })
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.sort_direction = match this.sort_direction {
                            SortDirection::Ascending => SortDirection::Descending,
                            SortDirection::Descending => SortDirection::Ascending,
                        }; this.apply_filters(); cx.notify();
                    })))
                .child(button::Button::new("theme")
                    .label(if self.dark_mode { "☀" } else { "🌙" })
                    .on_click(cx.listener(|this, _, _, cx| { this.dark_mode = !this.dark_mode; cx.notify(); })))
                .child(button::Button::new("settings-btn").label("⚙")
                    .on_click(cx.listener(|this, _, _, cx| { this.show_settings = !this.show_settings; cx.notify(); })))
                .child(button::Button::new("del-btn").label("🗑")
                    .on_click(cx.listener(|this, _, _, cx| { this.delete_selected(); cx.notify(); }))))
            .child(div().flex_1().bg(if self.dark_mode { rgb(0x222222) } else { rgb(0xf0f0f0) })
                .overflow_y_scrollbar()
                .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                    if this.search_focused {
                        match event.keystroke.key.as_str() {
                            "enter" => { this.search_focused = false; cx.notify(); }
                            "escape" => { this.search_focused = false; this.search_text.clear(); this.apply_filters(); cx.notify(); }
                            "backspace" => { this.search_text.pop(); this.apply_filters(); cx.notify(); }
                            k if k.len() == 1 => { this.search_text.push_str(k); this.apply_filters(); cx.notify(); }
                            _ => {}
                        }
                        return;
                    }
                    match event.keystroke.key.as_str() {
                        "right" | "down" => { if this.focused_index + 1 < this.filtered_indices.len() { this.focused_index += 1; cx.notify(); } }
                        "left" | "up" => { this.focused_index = this.focused_index.saturating_sub(1); cx.notify(); }
                        "space" => { this.toggle_select_focused(); cx.notify(); }
                        "delete" => { this.delete_selected(); cx.notify(); }
                        "escape" => { this.menu_visible = false; this.selected_indices.clear(); cx.notify(); }
                        _ => {}
                    }
                }))
                .child(div().p(px(4.)).flex().flex_wrap().gap(px(8.)).children(grid_items)))
            .child(div().h(px(22.)).bg(surf).border_t_1().border_color(border)
                .px_2().text_xs().text_color(rgb(0x888888)).flex_row().items_center()
                .child(format!("{} / {} · Sel: {}", self.filtered_indices.len(), self.captures.len(), self.selected_indices.len())))
    }

    fn preview(&mut self, surf: Rgba) -> impl IntoElement {
        let cap = self.filtered_indices.get(self.focused_index).and_then(|&i| self.captures.get(i));
        let info = cap.map(|c| (c.primary_path.clone(), c.base_name.clone(), c.primary_format.clone(), c.file_size, c.has_xmp, c.stack_count));
        div().w(px(320.)).h_full().bg(surf).border_l_1().border_color(rgb(0xe0e0e0)).flex_col()
            .child(if let Some((path, name, fmt, fsize, xmp, stack)) = info {
                let tex = self.textures.get_or_load(&path, 600);
                div().flex_1().flex_col().overflow_hidden()
                    .child(div().flex_1().bg(rgb(0x1a1a1a)).flex().items_center().justify_center()
                        .child(match tex { Some(src) => img(src).object_fit(ObjectFit::Contain).w_full().h_full().into_any(), None => div().child("Loading...").into_any() }))
                    .child(div().bg(surf).p_2().text_xs().flex_col()
                        .child(div().font_weight(FontWeight::BOLD).text_sm().child(name))
                        .child(format!("{} · {} KB", fmt, fsize.map(|s| s / 1024).unwrap_or(0)))
                        .child(format!("Stack: {} · XMP: {}", stack, if xmp { "Yes" } else { "No" })))
                    .into_any()
            } else { div().flex_1().flex().items_center().justify_center().text_sm().text_color(rgb(0xbbbbbb)).child("No image selected").into_any() })
    }

    fn context_menu(&mut self, surf: Rgba, border: Rgba, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.menu_visible { return div().into_any(); }
        let cap_path = self.menu_target.and_then(|i| self.captures.get(i)).map(|c| c.primary_path.clone());
        div().absolute().left(self.menu_pos.x).top(self.menu_pos.y)
            .bg(surf).border_1().border_color(border).rounded_md().shadow_lg()
            .text_sm().flex_col().min_w(px(160.)).py(px(2.))
            .child(div().px_3().py(px(4.)).hover(|s| s.bg(rgb(0xe5e5e5))).cursor_pointer()
                .on_mouse_down(MouseButton::Left, cx.listener(|this, _, _, cx| { this.delete_selected(); this.menu_visible = false; cx.notify(); }))
                .child("🗑 Delete"))
            .child(if let Some(ref cp) = cap_path {
                let p = cp.clone();
                div().px_3().py(px(4.)).hover(|s| s.bg(rgb(0xe5e5e5))).cursor_pointer()
                    .on_mouse_down(MouseButton::Left, cx.listener(move |this, _, _, cx| {
                        if let Err(e) = std::process::Command::new("explorer").arg("/select,").arg(&p).spawn() {
                            eprintln!("explorer: {}", e);
                        }
                        this.menu_visible = false; cx.notify();
                    }))
                    .child("📂 Show in Explorer").into_any()
            } else { div().into_any() })
            .child(div().h(px(1.)).bg(border))
            .child(div().h(px(1.)).bg(border))
            .into_any()
    }

    fn settings_dialog(&mut self, surf: Rgba, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.show_settings { return div().into_any(); }
        let size = self.thumbnail_size;
        div().absolute().inset_0().bg(rgba(0)).flex().items_center().justify_center()
            .child(div().bg(surf).rounded_lg().shadow_xl().p_4().flex_col().gap(px(12.)).min_w(px(300.))
                .child(div().text_sm().font_weight(FontWeight::BOLD).flex_row().justify_between()
                    .child("Settings").child(button::Button::new("close-set").label("✕")
                        .on_click(cx.listener(|this, _, _, cx| { this.show_settings = false; cx.notify(); }))))
                .child(div().text_xs().child(format!("Thumbnail Size: {}px", size)))
                .child(div().flex_row().gap(px(4.)).children(
                    vec![120u32, 160, 200, 220, 260, 300, 400].into_iter().map(|s| {
                        let sel = s == size;
                        button::Button::new(format!("sz-{}", s)).label(if sel { format!("◉ {}", s) } else { format!("○ {}", s) })
                            .on_click(cx.listener(move |this, _, _, cx| { this.thumbnail_size = s; cx.notify(); }))
                    }).collect::<Vec<_>>()
                ))
                .child(div().h(px(1.)).bg(rgb(0xe0e0e0)))
                .child(div().flex_row().items_center().gap(px(8.))
                    .child("Theme:").child(
                        button::Button::new("theme-dlg")
                            .label(if self.dark_mode { "☀ Light" } else { "🌙 Dark" })
                            .on_click(cx.listener(|this, _, _, cx| { this.dark_mode = !this.dark_mode; cx.notify(); }))
                    )))
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
