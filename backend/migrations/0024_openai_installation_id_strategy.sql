-- OpenAI installation_id 派生策略：per-account 每账号独立随机 UUIDv4（现状）；
-- egress-grouped 在凭据创建时按出口分组确定性派生，同一出口（直连一组、
-- 同一 outbound_proxies 记录一组）的账号共享同一 installation_id，对齐官方
-- 「一台设备（CODEX_HOME）多账号切换」的分布。默认保持 per-account：
-- 存量账号的 installation_id 已被上游观测，切换策略不重写任何存量凭据，
-- 仅影响切换后新创建的凭据（迁移语义见 .pi/coordination/P2-3-DESIGN.md）。
alter table runtime_settings
    add column openai_installation_id_strategy text not null default 'per-account'
        check (openai_installation_id_strategy in ('per-account', 'egress-grouped'));
