<script setup lang="ts">
import type { ClientProfileSelection, ProviderRequestProfile, XaiClientProfileSelection } from '@/api/modules/client-profiles'
import { computed } from 'vue'
import ClientProfileEditor from '@/components/client-profile/ClientProfileEditor.vue'
import XaiClientProfileEditor from '@/components/client-profile/XaiClientProfileEditor.vue'

// 账号归属唯一 Provider；未选择平台或混合导入时不提供账号级画像
withDefaults(defineProps<{
  provider?: string
  active?: boolean
  disabled?: boolean
}>(), {
  provider: undefined,
  active: true,
  disabled: false,
})

const model = defineModel<ProviderRequestProfile | null>({ required: true })
const openai = computed<ClientProfileSelection | null>({
  get: () => (model.value as ClientProfileSelection | null) ?? null,
  set: value => (model.value = value as ProviderRequestProfile | null),
})
const xai = computed<XaiClientProfileSelection | null>({
  get: () => (model.value as XaiClientProfileSelection | null) ?? null,
  set: value => (model.value = value as ProviderRequestProfile | null),
})
</script>

<template>
  <ClientProfileEditor
    v-if="provider === 'openai'"
    v-model="openai"
    allow-inherit
    inherit-label="跟随调用方"
    :active="active"
    :disabled="disabled"
  />
  <XaiClientProfileEditor
    v-else-if="provider === 'xai'"
    v-model="xai"
    allow-inherit
    inherit-label="跟随调用方"
    :active="active"
    :disabled="disabled"
  />
</template>
