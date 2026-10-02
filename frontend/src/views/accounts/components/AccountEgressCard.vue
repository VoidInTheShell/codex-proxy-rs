<script setup lang="ts">
import type { EgressDistribution, EgressGroup } from '@/api'
import { BaseCard, BasePopover } from '@codex-proxy/ui'

import { TriangleAlert, Users } from '@lucide/vue'
import { computed } from 'vue'
import { formatProviderLabel } from '@/utils/providers'

const props = defineProps<{
  distribution: EgressDistribution
}>()

const groups = computed(() => props.distribution.groups)

function groupLabel(group: EgressGroup): string {
  return group.kind === 'direct' ? '直连出口' : group.name ?? '代理出口'
}

function locationText(group: EgressGroup): string {
  const location = group.location
  return location ? `${location.city} · ${location.timezone}` : ''
}
</script>

<template>
  <BaseCard
    title="出口分布"
    description="账号按出口分组，共享出口达到阈值时标记提醒"
  >
    <div class="flex flex-wrap items-center gap-2">
      <BasePopover
        v-for="group in groups"
        :key="group.proxyId ?? 'direct'"
        trigger="hover-click"
        placement="bottom-start"
      >
        <template #trigger="{ open }">
          <button
            type="button"
            class="inline-flex max-w-72 cursor-pointer items-center gap-1.5 rounded-cp-sm border-0 px-2.5 py-1.5 text-cp-sm leading-none font-emphasis outline-none transition-colors focus-visible:ring-2 focus-visible:ring-cp-control-outline motion-reduce:transition-none"
            :class="group.alerting
              ? 'bg-cp-warning-container text-cp-warning-on-container'
              : 'bg-cp-fill-quaternary text-cp-text-secondary'"
            :aria-label="`${groupLabel(group)}：${group.accountCount} 个账号`"
            :aria-expanded="open"
            aria-haspopup="dialog"
          >
            <TriangleAlert
              v-if="group.alerting"
              class="size-3.5 shrink-0"
              aria-hidden="true"
            />
            <span class="truncate">{{ groupLabel(group) }}</span>
            <span class="font-mono tabular-nums">{{ group.accountCount }}</span>
          </button>
        </template>
        <section class="w-88 overflow-hidden rounded-cp-lg">
          <header class="flex items-start gap-3 bg-cp-popover-header-bg px-4 py-3">
            <span
              class="inline-flex size-8 shrink-0 items-center justify-center rounded-lg"
              :class="group.alerting
                ? 'bg-cp-warning-container text-cp-warning-on-container'
                : 'bg-cp-fill-quaternary text-cp-text-secondary'"
            >
              <TriangleAlert v-if="group.alerting" class="size-4" aria-hidden="true" />
              <Users v-else class="size-4" aria-hidden="true" />
            </span>
            <div class="min-w-0">
              <p class="m-0 truncate text-cp font-heavy text-cp-text">
                {{ groupLabel(group) }}
              </p>
              <p class="m-0 mt-1 text-cp-sm text-cp-text-secondary">
                {{ group.accountCount }} 个账号使用此出口
                <template v-if="group.alerting && distribution.alertEnabled">
                  ，达到共享提醒阈值（{{ distribution.alertThreshold }}）
                </template>
              </p>
            </div>
          </header>
          <div class="flex flex-col gap-2 px-4 py-3">
            <p v-if="group.endpoint" class="m-0 break-all font-mono text-cp-xs text-cp-text-secondary">
              {{ group.endpoint }}
            </p>
            <p v-if="locationText(group)" class="m-0 truncate text-cp-xs text-cp-text-secondary">
              {{ locationText(group) }}
            </p>
            <p v-if="group.exitIp" class="m-0 break-all font-mono text-cp-xs text-cp-text-secondary">
              出口 IP {{ group.exitIp }}
            </p>
            <ul class="m-0 flex list-none flex-col gap-1 p-0">
              <li
                v-for="account in group.accounts"
                :key="account.id"
                class="flex min-w-0 items-center justify-between gap-3 text-cp-sm"
              >
                <span class="m-0 truncate text-cp-text" :class="account.enabled ? '' : 'text-cp-text-quaternary'">
                  {{ account.name }}
                </span>
                <span class="m-0 shrink-0 text-cp-xs text-cp-text-quaternary">
                  {{ account.enabled ? formatProviderLabel(account.provider) : '已停用' }}
                </span>
              </li>
            </ul>
          </div>
        </section>
      </BasePopover>
    </div>
  </BaseCard>
</template>
