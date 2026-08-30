#!/usr/bin/env bash
# 安装 ftpt 应用图标到系统，使 KDE/GNOME Wayland 正确显示窗口图标（替换默认 "W" 图标）。
#
# 背景：Wayland 合成器（KWin/KDE）不通过 GTK set_icon() 设置窗口图标，而是根据
# 窗口的 xdg app_id 去 ~/.local/share/applications/ 找同名 .desktop 文件，读其
# Icon= 字段，再到图标主题找图标文件。
#
# 实测（WAYLAND_DEBUG=client 抓包）确认：Tauri/WebKit 窗口的 xdg app_id 是
# 「可执行文件名」，即 "ftpt"（tao#910：未显式设置 app_id 时默认用 executable 名）。
# 因此 KWin 会用 "ftpt" 去 ApplicationsLocation 找 ftpt.desktop。
#
# 重要：用户的 ~/.local/share/icons/hicolor 若缺 index.theme（本脚本前曾缺失），
# 图标主题系统会把它当成无效主题，导致 Icon=ftpt 解析失败 → 任务栏/Alt-Tab 显示空白。
# 本脚本会生成 hicolor 的 index.theme，并把图标装进 hicolor（默认回退主题）以及
# 当前 KDE 正在使用的主题（如 WhiteSur），双保险。
#
# 本脚本：
#   1. 生成/确保 hicolor/index.theme 存在（图标主题元数据，缺失则主题不被识别）
#   2. 把 icons/ 全套尺寸安装到 hicolor + 当前主题的 apps/ 目录
#   3. 创建 ftpt.desktop（主，匹配实测 app_id "ftpt"）与 com.ftpt.app.desktop（备）
#   4. 刷新图标缓存与 desktop 数据库

set -euo pipefail

# 仓库内 icons 目录（脚本位于 src-tauri/install-icon.sh，向上两级到项目根）
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ICON_SRC_DIR="$SCRIPT_DIR/icons"

# hicolor 图标主题安装根
ICON_DST="$HOME/.local/share/icons/hicolor"
# .desktop 安装目录
APPLICATION_DST="$HOME/.local/share/applications"

# 图标名（.desktop 的 Icon= 值；对应 hicolor 里的 <name>.png）
ICON_NAME="ftpt"

# 待复制进主题的尺寸（Tauri 已生成 icon-<size>.png）
SIZES="16 22 24 32 48 64 96 128 192 256 512"

# 当前 KDE 图标主题（从 kdeglobals 读，回退 hicolor）
CURRENT_THEME="$(kreadconfig6 --file kdeglobals --group Icons --key Theme 2>/dev/null || true)"
CURRENT_THEME="${CURRENT_THEME:-hicolor}"
THEME_DST="$HOME/.local/share/icons/$CURRENT_THEME"
echo "==> 当前 KDE 图标主题: $CURRENT_THEME"

# 生成 hicolor 的 index.theme（图标主题元数据；缺了会让 hicolor 不被当作有效主题）
ensure_hicolor_index() {
  local idx="$ICON_DST/index.theme"
  if [ -f "$idx" ]; then
    echo "==> hicolor/index.theme 已存在"
    return
  fi
  echo "==> 生成 hicolor/index.theme（缺失导致主题不被识别）"
  mkdir -p "$ICON_DST"
  local dirs=""
  for s in $SIZES; do dirs="${dirs}${dirs:+,}${s}x${s}/apps"; done
  {
    echo "[Icon Theme]"
    echo "Name=Hicolor"
    echo "Comment=Fallback icon theme"
    echo "Hidden=true"
    echo "Directories=$dirs"
    echo ""
    for s in $SIZES; do
      echo "[${s}x${s}/apps]"
      echo "Size=$s"
      echo "Context=Applications"
      echo "Type=Threshold"
      echo ""
    done
  } > "$idx"
}

# 装图标到某个主题目录。theme<dir> 是主题根；installed<size>/apps/ftpt.png 会被放置。
install_icons_to() {
  local theme_root="$1"
  local label="$2"
  echo "==> 安装图标到 ${label}: $theme_root"
  if [ ! -f "$theme_root/index.theme" ]; then
    echo "   (提示: $theme_root 缺 index.theme，跳过以避免产生未被索引的主题)"
    return
  fi
  for s in $SIZES; do
    local src="$ICON_SRC_DIR/icon-$s.png"
    [ -e "$src" ] || continue
    local dir="$theme_root/${s}x${s}/apps"
    # 某些主题（如 WhiteSur）目录命名用 apps/<size> 而非 <size>x<size>/apps
    if [ ! -d "$theme_root/${s}x${s}" ] && [ -d "$theme_root/apps/$s" ]; then
      dir="$theme_root/apps/$s"
    fi
    mkdir -p "$dir"
    cp "$src" "$dir/${ICON_NAME}.png"
    cp "$src" "$dir/${ICON_NAME}-${s}.png"
    echo "   -> $dir/${ICON_NAME}.png"
  done
  # scalable 目录（高清，供任务栏/大尺寸）
  if [ -d "$theme_root/apps/scalable" ]; then
    cp "$ICON_SRC_DIR/icon-256.png" "$theme_root/apps/scalable/${ICON_NAME}.png"
    echo "   -> apps/scalable/${ICON_NAME}.png"
  fi
}

ensure_hicolor_index
install_icons_to "$ICON_DST" "hicolor"

# 若当前主题不是 hicolor，也装一份（如 WhiteSur）
if [ "$CURRENT_THEME" != "hicolor" ] && [ -d "$THEME_DST" ]; then
  install_icons_to "$THEME_DST" "$CURRENT_THEME"
fi

echo "==> 创建 .desktop 文件"
mkdir -p "$APPLICATION_DST"
# Exec 优先指向仓库内调试二进制（开发期点击启动器可用）；打包安装时 bundler
# 会生成指向 /usr/bin/ftpt 的 .desktop，本脚本仅服务开发期 Wayland 窗口图标匹配。
FTPT_BIN="$(dirname "$SCRIPT_DIR")/../../target/debug/ftpt"
[ -x "$FTPT_BIN" ] || FTPT_BIN="ftpt"

# 实测（WAYLAND_DEBUG=client）显示 Tauri/WebKit 窗口的 xdg app_id 是「可执行文件名」，
# 即 "ftpt"（而非 identifier com.ftpt.app）。因此 KWin 会用 "ftpt" 去
# ApplicationsLocation 找 ftpt.desktop。据此创建 ftpt.desktop（主）与 com.ftpt.app.desktop（备）。
# StartupWMClass 应与窗口的 app_id/wmclass 对齐；此处都设为 ftpt。
write_desktop() {
  local fname="$1"
  local wmclass="$2"
  cat > "$APPLICATION_DST/${fname}.desktop" <<EOF
[Desktop Entry]
Type=Application
Name=ftpt
Comment=照片管理与筛选工具
GenericName=Photo Manager
Exec=$FTPT_BIN
Icon=$ICON_NAME
Terminal=false
Categories=Graphics;Photography;
StartupWMClass=$wmclass
StartupNotify=true
EOF
  echo "   -> ${fname}.desktop"
}

# 主：匹配实测 xdg app_id "ftpt"
write_desktop "ftpt" "ftpt"
# 备：匹配 identifier（若启用 enableGTKAppId 后 app_id 为 com.ftpt.app 的场合）
write_desktop "com.ftpt.app" "com.ftpt.app"

echo "==> 刷新图标缓存与 desktop 数据库"
if command -v gtk-update-icon-cache >/dev/null 2>&1; then
  gtk-update-icon-cache -f -t "$ICON_DST" 2>/dev/null || true
  if [ "$CURRENT_THEME" != "hicolor" ] && [ -d "$THEME_DST/index.theme" ]; then
    gtk-update-icon-cache -f -t "$THEME_DST" 2>/dev/null || true
  fi
fi
if command -v update-desktop-database >/dev/null 2>&1; then
  update-desktop-database "$APPLICATION_DST" 2>/dev/null || true
fi

echo "==> 完成。请注销重登（或重启 KWin）以刷新图标缓存与窗口 app_id 关联。"
