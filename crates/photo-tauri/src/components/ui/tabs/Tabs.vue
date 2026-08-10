<script setup lang="ts">
// shadcn-vue Tabs 根（reka-ui TabsRoot 封装）：v-model 绑定激活 tab。
// 注意：必须显式 v-bind="$attrs"——仅 v-bind="props" 时 onUpdate:modelValue
// 事件不会透传给 TabsRoot，点击 tab 后 emit 无处接收（data-state 不更新）。
import { TabsRoot, type TabsRootProps } from 'reka-ui'

const props = defineProps<TabsRootProps>()
</script>

<template>
  <TabsRoot v-bind="{ ...props, ...$attrs }">
    <slot />
  </TabsRoot>
</template>
