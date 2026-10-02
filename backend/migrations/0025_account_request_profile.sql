-- 账号级请求画像：每个上游账号可独立配置 wire 画像选择（预设或自定义 UA）。
-- NULL 表示不覆盖，继续按 client key 覆盖 > 全局默认 > Provider 内置默认解析。
alter table provider_accounts
    add column request_profile_json jsonb,
    add constraint provider_accounts_request_profile_object
        check (request_profile_json is null or jsonb_typeof(request_profile_json) = 'object');
