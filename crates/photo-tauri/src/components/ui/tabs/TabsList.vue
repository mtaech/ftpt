<script setup lang="ts">
// shadcn-vue TabsList：tab 导航容器（默认横向；竖排/无底色等用 class 覆盖，经 twMerge 合并）
import { TabsList, type TabsListProps } from 'reka-ui'
import { cn } from '@/lib/utils'

const props = defineProps<TabsListProps & { class?: string }>()
</script>

<template>
  <TabsList
    v-bind="props"
    :class="
      cn('inline-flex items-center justify-center rounded-lg bg-muted p-1 text-muted-foreground', props.class)
    "
  >
    <slot />
  </TabsList>
</template>
