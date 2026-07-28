# 评分/旗标放弃 XMP 旁车，由 SQLite 独占

评分、颜色标签、旗标不再写入或读取 `.xmp` 旁车文件，改为全部持久化到文件夹数据库（`.pt/data.db`）的 `xmp_meta` 表。`xmp_meta` 从缓存表升格为真相表，与 recognition 同级保护——不可被缓存清理操作触碰。

## 决策动机

- **可逆性低**：已有用户在照片目录中通过本应用打过评分/旗标（写入 .xmp 旁车），切换后磁盘上的 .xmp 数据直接作废，不迁移。对用户而言不可逆。
- **令人惊讶**：照片管理软件通常期望 .xmp/Lightroom interop。未来维护者会疑惑为什么评分不走旁车。答案：本应用是个人鸟类照片 culling 工具，非多软件协作 workflow；SQLite 单轨简化了文件操作同步（旁车文件必须跟随移动复制删除，是 ops 层最大的复杂度来源之一）。
- **真实权衡**：两种替代方案都被考虑过——（1）迁移 .xmp 数据后删除文件，破坏性太强且无回退；（2）保留双轨读，让写仍然产生 .xmp，但 ops 同步开销持续存在。最终选择彻底切除：源码简单，用户从零开始。

## 后果

- scanner 不再将 .xmp 配对为 sidecar（is_sidecar 字段从 SourceFile 删除，sidecar_extensions 配置项删除）
- ops 移动/复制/删除不再携带 .xmp 文件（删除图片会留下孤儿 .xmp）
- app 评分/旗标 setter 改为 `db.get_xmp` + `db.put_xmp`，无 db 时操作静默丢弃
- xmp.rs 模块整体删除（6 个测试随删）；cache.rs 中 dead xmp 代码一并清理
- 文件操作需要同步 xmp_meta 行（对齐 recognition 的 sync_delete/rename/copy 模式）
