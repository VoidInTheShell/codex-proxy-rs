export const rotationOptions = [
  {
    label: '智能调度（推荐）',
    value: 'smart',
    description: '综合负载、剩余额度、健康和延迟评分，支持自定义偏好与权重回切',
  },

  {
    label: '额度重置优先',
    value: 'quota_reset_priority',
    description: '优先选择额度窗口更快重置的账号，适合在重置前消耗剩余额度',
  },
  {
    label: '轮询调度',
    value: 'round_robin',
    description: '在可用候选账号间按顺序轮转，分配结果最可预测',
  },
  {
    label: '粘性调度',
    value: 'sticky',
    description: '优先复用最近使用的账号，直到不可用后再切换',
  },
] as const

export const installationIdOptions = [
  {
    label: '账号独立',
    value: 'per-account',
    description: '每个账号生成独立 installation_id，与官方 Codex CLI 单设备多账号行为一致',
  },
  {
    label: '出口分组',
    value: 'egress-grouped',
    description: '同一出口（直连或同代理 URL）共享稳定 installation_id，多账号对外表现为同一台设备',
  },
] as const
