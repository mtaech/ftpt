<script setup lang="ts">
// shadcn-vue TabsTrigger：单个 tab 项（data-[state=active] 高亮由语义变量驱动，可经 class 覆盖）
import { TabsTrigger, type TabsTriggerProps } from 'reka-ui'
import { cn } from '@/lib/utils'

const props = defineProps<TabsTriggerProps & { class?: string }>()
</script>

<template>
  <TabsTrigger
    v-bind="props"
    :class="
      cn(
        'inline-flex items-center justify-center whitespace-nowrap rounded-lg px-3 py-1 m3-label-large ring-offset-background transition-[background-color,border-color,color,box-shadow] duration-100 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 disabled:pointer-events-none disabled:opacity-50 data-[state=active]:bg-secondary-container data-[state=active]:text-on-secondary-container data-[state=active]:shadow-none',
        props.class,
      )
    "
  >
    <slot />
  </TabsTrigger>
</template>
