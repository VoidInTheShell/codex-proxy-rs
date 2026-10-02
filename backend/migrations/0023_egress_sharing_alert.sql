-- 出口共享提醒：账号 × 出口分布视图按同出口账号数标记风险分组，只做
-- 提示不拦截绑定（绑定是运维决策，一次导入多账号绑同一代理是合法用法）。
-- 默认阈值 2：同出口达到 2 个账号即为真实共享，与部署实测的最高风险暴露
-- （成对账号共享出口 IP）一一对应；下限 2 保证单账号独占出口不会被标记。
alter table runtime_settings
    add column egress_sharing_alert_enabled boolean not null default true,
    add column egress_sharing_alert_threshold bigint not null default 2
        check (egress_sharing_alert_threshold between 2 and 1000);
