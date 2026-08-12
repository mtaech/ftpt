// 选中态：多选 + 锚点语义（Phase 2 自 GPUI metadata.rs select 移植）。
// 兼容旧单选形态：selectedIndex / selected / selectedPaths / select / move / clear。
import { defineStore } from 'pinia'
import type { CaptureMeta } from '@/lib/bindings'
import { useCapturesStore } from './captures'
import { useFilterStore } from './filter'

/** 下标合法性检查：越界返回 null（单击/切换/范围共用） */
function validIndex(i: number): number | null {
  const n = useCapturesStore().items.length
  return i >= 0 && i < n ? i : null
}

export const useSelectionStore = defineStore('selection', {
  state: () => ({
    /** 选中集：有序（网格顺序）、去重；空数组 = 无选中 */
    selectedIndices: [] as number[],
    /** 锚点：Shift 范围基准 / 方向键焦点；主选中项优先读它 */
    anchorIndex: null as number | null,
  }),
  getters: {
    /** 主选中项下标：锚点优先，回退选中集末尾（兼容旧单选 selectedIndex） */
    selectedIndex(): number | null {
      return this.anchorIndex ?? this.selectedIndices.at(-1) ?? null
    },
    /** 主选中拍摄（越界/空列表安全；PhotoPreview current 读取它） */
    selected(): CaptureMeta | null {
      const captures = useCapturesStore()
      const i = this.selectedIndex
      if (i === null) return null
      return captures.items[i] ?? null
    },
    /** 全部选中路径（command 入参形态） */
    selectedPaths(): string[] {
      const captures = useCapturesStore()
      return this.selectedIndices
        .map((i) => captures.items[i]?.primaryPath)
        .filter((p): p is string => Boolean(p))
    },
    /** 堆叠组总数（网格显示项数；Home/End 导航上界） */
    stackCount(): number {
      return useFilterStore().stackGroups.length
    },
  },
  actions: {
    /** 单击：替换为 [i]，anchor = i */
    select(i: number) {
      if (validIndex(i) === null) return
      this.selectedIndices = [i]
      this.anchorIndex = i
    },
    /** Ctrl+单击：切换 i；anchor = i（移除锚点时 anchor 挪到选中集末尾或 null） */
    toggle(i: number) {
      if (validIndex(i) === null) return
      const pos = this.selectedIndices.indexOf(i)
      if (pos >= 0) {
        // 移除：保持有序
        this.selectedIndices.splice(pos, 1)
        if (this.anchorIndex === i) {
          this.anchorIndex = this.selectedIndices.at(-1) ?? null
        }
      } else {
        // 插入：按网格顺序
        const insertAt = this.selectedIndices.findIndex((x) => x > i)
        if (insertAt < 0) this.selectedIndices.push(i)
        else this.selectedIndices.splice(insertAt, 0, i)
        this.anchorIndex = i
      }
    },
    /** Shift+单击：anchor→i 闭区间替换选中集，anchor 不变（无锚点时退化为单击选中） */
    selectRange(i: number) {
      if (validIndex(i) === null) return
      const anchor = this.anchorIndex
      if (anchor === null) {
        this.select(i)
        return
      }
      const lo = Math.min(anchor, i)
      const hi = Math.max(anchor, i)
      this.selectedIndices = Array.from({ length: hi - lo + 1 }, (_, k) => lo + k)
      // anchor 不变（契约：范围基准保持）
    },
    isSelected(i: number): boolean {
      return this.selectedIndices.includes(i)
    },
    /** 方向键相对移动：折叠为单选中项（清多选），anchor = 新项；空列表 no-op */
    move(delta: number) {
      const n = useCapturesStore().items.length
      if (n === 0) return
      const cur = this.selectedIndex ?? (delta > 0 ? -1 : 0)
      this.moveTo(cur + delta)
    },
    /** 绝对移动（Home/End/方向键共用）：折叠为单选中项（清多选），anchor = index，越界钳制 */
    moveTo(i: number) {
      const n = useCapturesStore().items.length
      if (n === 0) return
      const clamped = Math.min(Math.max(i, 0), n - 1)
      this.selectedIndices = [clamped]
      this.anchorIndex = clamped
    },
    /** Ctrl+A：全选（有序）；anchor 置首项（后续 Shift+单击范围语义明确） */
    selectAll() {
      const n = useCapturesStore().items.length
      if (n === 0) return
      this.selectedIndices = Array.from({ length: n }, (_, k) => k)
      this.anchorIndex = 0
    },
    clear() {
      this.selectedIndices = []
      this.anchorIndex = null
    },
    /**
     * 堆叠切换：以指定成员所在堆叠组为上下文循环切换激活成员并选中（±1 循环）。
     * 网格堆叠徽标点击与预览工具条共用；组不存在/单成员组 no-op。
     */
    cycleStackFrom(i: number, direction: 1 | -1) {
      const filter = useFilterStore()
      const g = filter.stackGroups.find((gr) => gr.members.includes(i))
      if (!g) return
      const next = filter.cycleStack(g.key, direction)
      if (next !== null) this.select(next)
    },
    /** 方向键组间导航：±1 移动一个堆叠组（选中目标组主成员/激活成员） */
    moveInStacks(delta: number) {
      const filter = useFilterStore()
      const groups = filter.stackGroups
      if (groups.length === 0) return
      const cur = this.selectedIndex
      let pos = cur === null ? -1 : filter.stackPositionOf(cur)
      if (pos < 0) pos = delta > 0 ? -1 : 0
      this.moveToStack(pos + delta)
    },
    /** 绝对组导航（Home/End 共用）：跳到第 pos 个堆叠组并选中其激活成员，越界钳制 */
    moveToStack(pos: number) {
      const groups = useFilterStore().stackGroups
      if (groups.length === 0) return
      this.select(groups[Math.min(Math.max(pos, 0), groups.length - 1)].active)
    },
  },
})
