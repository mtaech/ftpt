<script setup lang="ts">
import { useUiStore } from '@/stores/ui'
const ui = useUiStore()
</script>

<template>
  <div class="layout">
    <div class="layout__left" :style="{ width: ui.leftPanelWidth + 'px' }">
      <slot name="left" />
    </div>
    <div class="layout__center">
      <slot name="center" />
    </div>
    <div v-if="ui.rightPanelVisible" class="layout__right">
      <slot name="right" />
    </div>
  </div>
</template>

<style scoped>
.layout {
  display: flex;
  height: 100%;
  width: 100%;
  overflow: hidden;
}

.layout__left {
  flex-shrink: 0;
  border-right: 1px solid var(--border);
  background: var(--bg-surface);
  overflow-y: auto;
}

.layout__center {
  flex: 1 1 0;
  overflow: hidden;
  display: flex;
  flex-direction: column;
  min-width: 0;
}

.layout__right {
  flex: 1 1 0;
  min-width: 320px;
  border-left: 1px solid var(--border);
  background: var(--bg-surface);
  overflow-y: auto;
}
</style>
