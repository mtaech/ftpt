#!/usr/bin/env bash
# cargo 的 rustc-wrapper：把 rustc（及其派生的链接器）降到最低优先级，
# 让 cargo build 在后台编译时不再抢占前台（IDE / 浏览器 / 视频）的 CPU 与磁盘。
# 由 .cargo/config.toml 的 [build] rustc-wrapper 自动调用，无需手动使用。
# 缺 nice/ionice 时退化为直接 exec，绝不因为包装器本身让构建失败。
set -euo pipefail

if ! command -v nice >/dev/null 2>&1; then
  exec "$@"
fi

if command -v ionice >/dev/null 2>&1; then
  # -c 2 -n 7 = best-effort 最低档（不用 -c 3 idle：构建会被任意 I/O 饿死）
  exec nice -n 19 ionice -c 2 -n 7 "$@"
fi

exec nice -n 19 "$@"
