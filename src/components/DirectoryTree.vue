<script setup lang="ts">
import { ref, computed } from 'vue'
import { useBrowseStore } from '@/stores/browse'
import { expandDirectory, openFolderDialog } from '@/types/tauri'
import type { TreeNode } from '@/types'
import { Folder, FolderOpen, FolderPlus, ChevronRight } from 'lucide-vue-next'

const browse = useBrowseStore()

async function openFolder() {
  const dir = await openFolderDialog('选择照片目录')
  if (dir) browse.openDirectory(dir)
}

const expanded = ref<Set<string>>(new Set())
const expandedChildren = ref<Map<string, TreeNode[]>>(new Map())

const treeItems = computed(() => {
  const items: Array<{ node: TreeNode; depth: number }> = []

  function addNodes(nodes: TreeNode[], depth: number) {
    for (const node of nodes) {
      items.push({ node, depth })
      if (expanded.value.has(node.path)) {
        const children = expandedChildren.value.get(node.path)
        if (children && children.length > 0) {
          addNodes(children, depth + 1)
        }
      }
    }
  }

  addNodes(browse.directoryTree, 0)
  return items
})

async function toggle(node: TreeNode) {
  if (!node.hasChildren) return

  if (expanded.value.has(node.path)) {
    const s = new Set(expanded.value)
    s.delete(node.path)
    expanded.value = s
    return
  }

  try {
    const children = await expandDirectory(node.path)
    const m = new Map(expandedChildren.value)
    m.set(node.path, children)
    expandedChildren.value = m
    const s = new Set(expanded.value)
    s.add(node.path)
    expanded.value = s
  } catch {}
}

function select(path: string) {
  browse.openDirectory(path)
}

function isSelected(path: string) {
  return browse.currentPath === path
}
</script>

<template>
  <div class="tree">
    <div class="tree__header">
      <Folder :size="14" class="tree__header-icon" />
      <span>目录</span>
    </div>
    <div class="tree__actions">
      <button class="tree__open-btn" @click="openFolder">
        <FolderPlus :size="14" />
        打开目录…
      </button>
    </div>
    <div class="tree__list">
      <div v-if="treeItems.length === 0" class="tree__empty">无目录数据</div>
      <div
        v-for="{ node, depth } in treeItems"
        :key="node.path"
        class="tree-row"
        :class="{ 'tree-row--selected': isSelected(node.path) }"
        :style="{ paddingLeft: 8 + depth * 16 + 'px' }"
      >
        <span class="tree-toggle" @click="toggle(node)">
          <ChevronRight v-if="node.hasChildren" :size="10" class="tree-arrow" :class="{ 'tree-arrow--open': expanded.has(node.path) }" />
          <span v-else class="tree-dot" />
        </span>
        <span class="tree-label" @click="select(node.path)" :title="node.name">
          <Folder v-if="!expanded.has(node.path) && !isSelected(node.path)" :size="14" class="tree-folder" />
          <FolderOpen v-else :size="14" class="tree-folder" />
          {{ node.name }}
        </span>
      </div>
    </div>
  </div>
</template>

<style scoped>
.tree { display: flex; flex-direction: column; height: 100%; }
.tree__header { display: flex; align-items: center; gap: 8px; padding: 14px 16px 10px; font-family: var(--font-heading); font-size: 13px; font-weight: 600; color: var(--text-muted); letter-spacing: 0.02em; border-bottom: 1px solid var(--border-light); }
.tree__header-icon { color: var(--text-muted); }

.tree__actions { padding: 8px 12px; border-bottom: 1px solid var(--border-light); }
.tree__open-btn { display: flex; align-items: center; gap: 6px; width: 100%; font-family: var(--font-body); font-size: 12px; font-weight: 500; padding: 6px 10px; border: 1px solid var(--border); border-radius: var(--radius-sm); background: var(--bg-page); color: var(--text-secondary); cursor: pointer; transition: all var(--transition-fast); }
.tree__open-btn:hover { border-color: var(--primary); color: var(--primary); background: var(--primary-subtle); }

.tree__list { flex: 1; overflow-y: auto; padding: 4px; }
.tree__empty { padding: 16px; text-align: center; font-size: 13px; color: var(--text-muted); }
.tree-row { display: flex; align-items: center; gap: 2px; padding: 6px 8px; border-radius: var(--radius-sm); cursor: pointer; user-select: none; transition: background var(--transition-fast); }
.tree-row:hover { background: var(--bg-hover); }
.tree-row--selected { background: var(--primary-subtle); box-shadow: inset 2px 0 0 var(--primary); }
.tree-row--selected:hover { background: var(--primary-subtle); }
.tree-row--selected .tree-label { color: var(--primary); font-weight: 500; }
.tree-toggle { width: 18px; flex-shrink: 0; display: flex; align-items: center; justify-content: center; cursor: pointer; color: var(--text-muted); border-radius: 4px; padding: 1px; }
.tree-toggle:hover { background: var(--bg-hover); color: var(--text); }
.tree-arrow { transition: transform var(--transition-fast); }
.tree-arrow--open { transform: rotate(90deg); }
.tree-dot { width: 4px; height: 4px; border-radius: 50%; background: var(--border); display: block; }
.tree-label { flex: 1; display: flex; align-items: center; gap: 6px; font-size: 13px; color: var(--text-secondary); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; min-width: 0; }
.tree-folder { flex-shrink: 0; color: var(--text-muted); transition: color var(--transition-fast); }
.tree-row:hover .tree-folder { color: var(--text-secondary); }
.tree-row--selected .tree-folder { color: var(--primary); }
</style>
