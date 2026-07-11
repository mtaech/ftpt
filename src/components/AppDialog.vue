<script setup lang="ts">
import { X } from 'lucide-vue-next'

defineProps<{
  title: string
  width?: string
}>()

const emit = defineEmits<{ close: [] }>()
</script>

<template>
  <Teleport to="body">
    <Transition name="overlay">
      <div class="overlay" @click.self="emit('close')">
        <div class="dialog" :style="{ width: width || '480px' }">
          <div class="dialog__header">
            <h2 class="dialog__title">{{ title }}</h2>
            <button class="dialog__close" @click="emit('close')">
              <X :size="16" />
            </button>
          </div>
          <div class="dialog__body">
            <slot name="body" />
          </div>
          <div class="dialog__footer">
            <slot name="footer" />
          </div>
        </div>
      </div>
    </Transition>
  </Teleport>
</template>

<style scoped>
.overlay {
  position: fixed;
  inset: 0;
  z-index: 900;
  background: rgba(28, 25, 23, 0.4);
  display: flex;
  align-items: center;
  justify-content: center;
}

.dialog {
  background: var(--bg-elevated);
  border-radius: var(--radius-xl);
  box-shadow: var(--shadow-xl);
  transition: transform 200ms cubic-bezier(0.4, 0, 0.2, 1), opacity 200ms ease;
}

.dialog__header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 16px 20px;
  border-bottom: 1px solid var(--border-light);
}

.dialog__title {
  font-family: var(--font-heading);
  font-size: 16px;
  font-weight: 600;
  color: var(--text);
}

.dialog__close {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 28px;
  height: 28px;
  border: none;
  background: none;
  color: var(--text-muted);
  border-radius: var(--radius-sm);
  cursor: pointer;
  transition: all var(--transition-fast);
}

.dialog__close:hover {
  background: var(--bg-hover);
  color: var(--text);
}

.dialog__body {
  padding: 16px 20px;
  display: flex;
  flex-direction: column;
  gap: 12px;
  max-height: 60vh;
  overflow-y: auto;
}

.dialog__footer {
  display: flex;
  justify-content: flex-end;
  gap: 8px;
  padding: 12px 20px;
  border-top: 1px solid var(--border-light);
}

.overlay-enter-active,
.overlay-leave-active {
  transition: opacity 200ms ease;
}

.overlay-enter-from,
.overlay-leave-to {
  opacity: 0;
}

.overlay-enter-active .dialog,
.overlay-leave-active .dialog {
  transition: transform 200ms cubic-bezier(0.4, 0, 0.2, 1), opacity 200ms ease;
}

.overlay-enter-from .dialog {
  transform: scale(0.95);
  opacity: 0;
}

.overlay-leave-to .dialog {
  transform: scale(0.95);
  opacity: 0;
}
</style>
