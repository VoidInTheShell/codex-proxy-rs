<script setup lang="ts">
import { BaseCard, BaseForm, BaseFormItem, BaseInput, BaseSwitch } from '@codex-proxy/ui'

import { Users } from '@lucide/vue'

const enabled = defineModel<boolean>('enabled', { required: true })
const threshold = defineModel<string>('threshold', { required: true })
</script>

<template>
  <BaseCard
    title="出口共享提醒"
    description="账号按出口分组展示，同出口账号数达到阈值时标记提醒"
  >
    <BaseForm class="max-w-6xl sm:grid-cols-2">
      <BaseSwitch
        v-model="enabled"
        class="col-span-full justify-self-start"
        label="启用出口共享提醒"
        show-label
      />

      <BaseFormItem
        label="共享账号阈值"
        description="只提示不拦截绑定，调整绑定仍由管理员决定"
      >
        <BaseInput
          v-model="threshold"
          :disabled="!enabled"
          aria-label="共享账号阈值"
          type="number"
          min="2"
          max="1000"
          step="1"
        >
          <template #prefix>
            <Users class="size-4" />
          </template>
          <template #suffix>
            <span class="text-cp-sm">个</span>
          </template>
        </BaseInput>
      </BaseFormItem>
    </BaseForm>
  </BaseCard>
</template>
