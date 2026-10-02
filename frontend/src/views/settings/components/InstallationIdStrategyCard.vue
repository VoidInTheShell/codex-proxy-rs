<script setup lang="ts">
import type { installationIdOptions } from '../constants'
import { BaseCard } from '@codex-proxy/ui'
import { computed } from 'vue'

type InstallationIdOption = (typeof installationIdOptions)[number]
type InstallationIdStrategy = InstallationIdOption['value']

const props = defineProps<{
  options: readonly InstallationIdOption[]
  disabled: boolean
}>()

const model = defineModel<InstallationIdStrategy | ''>({ required: true })
const current = computed(() => props.options.find(option => option.value === model.value))
</script>

<template>
  <BaseCard title="installation_id 派生策略">
    <div class="grid max-w-6xl gap-3 sm:grid-cols-2">
      <div
        v-for="option in options"
        :key="option.value"
        class="relative flex"
      >
        <button
          type="button"
          class="min-h-25 w-full cursor-pointer rounded-cp border-0 px-4 py-3.5 text-left shadow-cp-input outline-none transition-[background-color,box-shadow,color] duration-160 focus-visible:ring-2 focus-visible:ring-cp-control-outline"
          :class="
            model === option.value
              ? 'bg-cp-control-item-bg-active text-cp-primary-text shadow-cp-tertiary'
              : 'bg-(--cp-input-bg,var(--cp-input-bg)) text-cp-text hover:bg-(--cp-input-hover-bg,var(--cp-input-hover-bg)) hover:shadow-cp-input-hover'
          "
          :aria-pressed="model === option.value"
          :disabled="disabled"
          @click="model = option.value"
        >
          <span class="flex items-center gap-2">
            <span
              class="inline-flex size-4 shrink-0 items-center justify-center rounded-full bg-cp-bg-container shadow-[inset_0_0_0_1px_var(--cp-color-border)]"
            >
              <span
                class="size-2 rounded-full transition-opacity duration-150"
                :class="model === option.value ? 'bg-cp-primary opacity-100' : 'opacity-0'"
              />
            </span>
            <span class="inline-flex items-baseline gap-1 text-cp leading-snug font-emphasis">
              {{ option.label }}
            </span>
          </span>
          <span class="mt-2 block text-cp leading-normal font-emphasis text-cp-text-secondary">
            {{ option.description }}
          </span>
        </button>
      </div>
    </div>
    <p
      v-if="current && current.value === 'egress-grouped'"
      class="m-0 mt-4 text-cp leading-normal text-cp-text-secondary"
    >
      策略只影响后续新增或重新导入的账号；已有账号的 installation_id 保持不变，需要按新策略重新导入才会更新。
    </p>
  </BaseCard>
</template>
