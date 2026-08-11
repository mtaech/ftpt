<script setup lang="ts">
// 自绘全局右键菜单（对齐 GPUI gpui_component PopupMenu）：
// store 驱动显隐与定位，Teleport 到 body 保证 z 覆盖预览工具栏等浮层；
// 点击外部 / Esc 关闭，hover 展开子菜单，菜单弹出时按自身尺寸钳制到视口内。
import { computed, nextTick, onMounted, onUnmounted, ref, watch } from 'vue'
import { CheckIcon, ChevronRightIcon } from '@lucide/vue'
import { useContextMenuStore, type ContextMenuItem } from '@/stores/contextMenu'

const menu = useContextMenuStore()

const rootEl = ref<HTMLElement | null>(null)
/** 当前展开的子菜单（父项下标，-1 = 无） */
const openSub = ref(-1)
/** 视口钳制后的定位（防止弹出时被屏幕边缘裁切） */
const pos = ref({ x: 0, y: 0 })

/** 渲染后按自身实际尺寸钳制到视口内（右下缘留 4px 余量） */
async function clampPosition() {
  await nextTick()
  const el = rootEl.value
  if (!el) return
  const rect = el.getBoundingClientRect()
  pos.value = {
    x: Math.min(menu.x, Math.max(window.innerWidth - rect.width - 4, 0)),
    y: Math.min(menu.y, Math.max(window.innerHeight - rect.height - 4, 0)),
  }
}

watch(
  () => menu.open,
  (open) => {
    openSub.value = -1
    if (open) void clampPosition()
  },
)

/** 子菜单展开方向：主菜单贴近右缘时向左展开，避免溢出视口 */
const subSide = computed(() =>
  window.innerWidth - pos.value.x < 320 ? 'right-full mr-1' : 'left-full ml-1',
)

/**
 * Esc 关闭。stopImmediatePropagation：菜单打开时吃掉按键，避免 keymap 的
 * Esc（退出预览）在同一事件里再次触发（菜单比 keymap 先注册，先执行）。
 */
function onKeydown(e: KeyboardEvent) {
  if (!menu.open) return
  if (e.key === 'Escape') {
    e.stopImmediatePropagation()
    menu.closeMenu()
  }
}
onMounted(() => window.addEventListener('keydown', onKeydown))
onUnmounted(() => window.removeEventListener('keydown', onKeydown))

/** 点击菜单项：执行动作后关闭（对齐 GPUI 菜单项点击即分发；子菜单/分隔线不可点） */
function run(item: ContextMenuItem) {
  if (item.kind === 'sep' || item.kind === 'submenu') return
  item.action()
  menu.closeMenu()
}
</script>

<template>
  <Teleport to="body">
    <!-- 全屏遮罩：点击外部关闭（同时挡住下方交互，右键也关闭） -->
    <div
      v-if="menu.open"
      class="fixed inset-0 z-[100]"
      @pointerdown="menu.closeMenu()"
      @contextmenu.prevent="menu.closeMenu()"
    />
    <!-- 菜单本体（z 高于遮罩；pointerdown.stop 防遮罩误关） -->
    <div
      v-if="menu.open"
      ref="rootEl"
      class="fixed z-[101] min-w-40 rounded-md border border-border bg-popover p-1 text-sm text-popover-foreground shadow-lg select-none"
      :style="{ left: pos.x + 'px', top: pos.y + 'px' }"
      @pointerdown.stop
    >
      <template v-for="(item, i) in menu.items" :key="i">
        <!-- 分隔线 -->
        <div v-if="item.kind === 'sep'" class="my-1 h-0.5 bg-border" />
        <!-- 子菜单：hover 展开右侧面板（此处用法只有一层嵌套） -->
        <div
          v-else-if="item.kind === 'submenu'"
          class="relative"
          @mouseenter="openSub = i"
          @mouseleave="openSub = -1"
        >
          <div
            class="flex items-center justify-between gap-4 rounded-sm px-2 py-1.5 hover:bg-accent"
          >
            <span>{{ item.label }}</span>
            <ChevronRightIcon class="size-3.5 shrink-0 text-muted-foreground" />
          </div>
          <div
            v-if="openSub === i"
            class="absolute top-0 z-10 min-w-36 rounded-md border border-border bg-popover p-1 shadow-md"
            :class="subSide"
          >
            <template v-for="(sub, j) in item.items" :key="j">
              <div v-if="sub.kind === 'sep'" class="my-1 h-0.5 bg-border" />
              <div
                v-else
                class="flex items-center justify-between gap-3 rounded-sm px-2 py-1.5 hover:bg-accent"
                @click="run(sub)"
              >
                <span :class="sub.kind === 'item' && sub.danger ? 'text-destructive' : ''">{{ sub.label }}</span>
                <CheckIcon
                  v-if="sub.kind === 'check' && sub.checked"
                  class="size-3.5 shrink-0 text-primary"
                />
              </div>
            </template>
          </div>
        </div>
        <!-- 普通项 / 勾选项 -->
        <div
          v-else
          class="flex items-center justify-between gap-3 rounded-sm px-2 py-1.5 hover:bg-accent"
          @click="run(item)"
        >
          <span :class="item.kind === 'item' && item.danger ? 'text-destructive' : ''">{{ item.label }}</span>
          <CheckIcon
            v-if="item.kind === 'check' && item.checked"
            class="size-3.5 shrink-0 text-primary"
          />
        </div>
      </template>
    </div>
  </Teleport>
</template>
