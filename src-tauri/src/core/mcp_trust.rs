//! 项目级 MCP 配置信任门（P0-1 / MCP-1 的纵深防御层）。
//!
//! 威胁模型：克隆的仓库可自带 `.aio/mcp-servers.json`（`autoStart: true` +
//! 任意 stdio command / http url）。信任标记必须保存在**仓库之外**（应用
//! 数据目录），否则恶意仓库可自声明信任。
//!
//! 键为 (规范化项目路径, server_id)，值为配置指纹——`transport` 的规范化
//! JSON 之 SHA-256。仓库内容一旦变更（换 command / args / env / url /
//! headers），指纹不匹配即自动失效，需重新确认。`BTreeMap` 的有序性保证
//! env/headers 键顺序变化不产生不同指纹。

use crate::core::fs_util;
use crate::core::models::McpServerConfig;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

const TRUST_FILE: &str = "mcp-trust.json";

#[derive(Serialize, Deserialize, Default, Debug)]
#[serde(rename_all = "camelCase")]
struct TrustFile {
    #[serde(default)]
    version: u32,
    /// 规范化项目路径 → (server_id → 配置指纹)
    #[serde(default)]
    entries: BTreeMap<String, BTreeMap<String, String>>,
}

/// 计算项目级服务器配置的指纹：只覆盖决定“执行什么 / 连哪里”的字段
/// （transport 全量）；id / autoStart / enabledTools 等展示与装配字段不参与。
pub fn config_fingerprint(cfg: &McpServerConfig) -> String {
    let canonical = serde_json::to_string(&cfg.transport).unwrap_or_default();
    let mut hasher = Sha256::new();
    hasher.update(canonical.as_bytes());
    let digest = hasher.finalize();
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// 项目路径规范化（信任键的组成部分）：物理路径优先，失败回退原样。
fn normalize_project_key(project_path: &str) -> String {
    std::fs::canonicalize(project_path)
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| project_path.to_string())
}

/// 信任存储（`$APPDATA/mcp-trust.json`，原子写）。
pub struct McpTrustStore {
    path: PathBuf,
}

/// 进程级互斥：串行化信任文件的读-改-写（与 secure_store 的 FALLBACK_MUTEX 同款
/// 范式）。无锁时两个并发 `set_trusted` 的后写覆盖先写 → 信任记录静默丢失
/// （失败回退为"未信任"，fail-safe 但用户操作白做）。
fn trust_write_mutex() -> &'static std::sync::Mutex<()> {
    static M: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
    M.get_or_init(|| std::sync::Mutex::new(()))
}

impl McpTrustStore {
    /// 打开（或延迟创建）应用数据目录下的信任存储。
    pub fn open(app_data: &Path) -> Self {
        Self {
            path: app_data.join(TRUST_FILE),
        }
    }

    /// 损坏 / 缺失时视为空存储（fail-open 到“全部未信任”，不 panic）。
    fn load(&self) -> TrustFile {
        std::fs::read_to_string(&self.path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    fn save(&self, file: &TrustFile) -> Result<(), String> {
        let bytes = serde_json::to_vec_pretty(file).map_err(|e| e.to_string())?;
        fs_util::atomic_write(&self.path, &bytes).map_err(|e| e.to_string())
    }

    /// 该项目级服务器是否已确认信任（指纹必须完全一致；配置变更自动失效）。
    pub fn is_trusted(&self, project_path: &str, server_id: &str, fingerprint: &str) -> bool {
        let key = normalize_project_key(project_path);
        self.load()
            .entries
            .get(&key)
            .and_then(|servers| servers.get(server_id))
            .is_some_and(|fp| fp == fingerprint)
    }

    /// 记录信任（原子写；读-改-写全程持进程锁，防并发丢更新）。
    pub fn set_trusted(
        &self,
        project_path: &str,
        server_id: &str,
        fingerprint: &str,
    ) -> Result<(), String> {
        let _guard = trust_write_mutex()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let key = normalize_project_key(project_path);
        let mut file = self.load();
        file.version = 1;
        file.entries
            .entry(key)
            .or_default()
            .insert(server_id.to_string(), fingerprint.to_string());
        self.save(&file)
    }

    /// 移除某项目下某 server 的信任条目（server 被删除时调用；不存在的条目为 no-op；
    /// 读-改-写全程持进程锁，防并发丢更新）。
    pub fn remove_trust(&self, project_path: &str, server_id: &str) -> Result<(), String> {
        let _guard = trust_write_mutex()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let key = normalize_project_key(project_path);
        let mut file = self.load();
        let Some(servers) = file.entries.get_mut(&key) else {
            return Ok(());
        };
        if servers.remove(server_id).is_none() {
            return Ok(());
        }
        if servers.is_empty() {
            file.entries.remove(&key);
        }
        self.save(&file)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::models::{McpServerConfig, McpTransport};
    use std::collections::BTreeMap;

    fn stdio_server(command: &str) -> McpServerConfig {
        let mut env = BTreeMap::new();
        env.insert("K".to_string(), "V".to_string());
        McpServerConfig {
            id: "srv".into(),
            display_name: "Server".into(),
            transport: McpTransport::Stdio {
                command: command.into(),
                args: vec!["--serve".into()],
                env,
                cwd: None,
            },
            enabled_tools: vec![],
            auto_start: false,
            has_stored_secret: false,
            trusted: false,
            from_catalog: None,
        }
    }

    fn temp_store(tag: &str) -> (McpTrustStore, PathBuf) {
        let dir =
            std::env::temp_dir().join(format!("aio-mcp-trust-{}-{}", tag, std::process::id()));
        std::fs::create_dir_all(&dir).expect("创建临时目录失败");
        (McpTrustStore::open(&dir), dir)
    }

    #[test]
    fn fingerprint_is_deterministic_and_env_order_insensitive() {
        let a = stdio_server("node");
        let b = stdio_server("node");
        assert_eq!(config_fingerprint(&a), config_fingerprint(&b));
        // autoStart 等非执行字段不参与指纹
        let mut c = stdio_server("node");
        c.auto_start = true;
        assert_eq!(config_fingerprint(&a), config_fingerprint(&c));
    }

    #[test]
    fn fingerprint_changes_when_command_changes() {
        let a = stdio_server("node");
        let b = stdio_server("powershell");
        assert_ne!(config_fingerprint(&a), config_fingerprint(&b));
    }

    #[test]
    fn trust_round_trip_and_fingerprint_mismatch() {
        let (store, dir) = temp_store("roundtrip");
        let fp = config_fingerprint(&stdio_server("node"));
        assert!(!store.is_trusted("/proj", "srv", &fp));
        store
            .set_trusted("/proj", "srv", &fp)
            .expect("写入信任失败");
        assert!(store.is_trusted("/proj", "srv", &fp));
        // 同 id 不同指纹（仓库改了 command）→ 未信任
        assert!(!store.is_trusted("/proj", "srv", "deadbeef"));
        // 不同项目互不影响
        assert!(!store.is_trusted("/other", "srv", &fp));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn corrupt_trust_file_treated_as_untrusted_everywhere() {
        let (store, dir) = temp_store("corrupt");
        std::fs::write(dir.join(TRUST_FILE), "not json at all").expect("准备坏文件失败");
        assert!(!store.is_trusted("/proj", "srv", "any"));
        // set_trusted 会整体重写，坏文件可自愈
        store
            .set_trusted("/proj", "srv", "fp")
            .expect("覆盖坏文件失败");
        assert!(store.is_trusted("/proj", "srv", "fp"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn remove_trust_drops_only_the_named_entry() {
        let (store, dir) = temp_store("remove");
        store.set_trusted("/proj", "a", "fa").expect("写入 a 失败");
        store.set_trusted("/proj", "b", "fb").expect("写入 b 失败");
        store
            .set_trusted("/other", "a", "fa")
            .expect("写入 other 失败");
        // 删除存在的条目 → 同项目其他条目与其他项目不受影响
        store.remove_trust("/proj", "a").expect("删除失败");
        assert!(!store.is_trusted("/proj", "a", "fa"));
        assert!(store.is_trusted("/proj", "b", "fb"));
        assert!(store.is_trusted("/other", "a", "fa"));
        // 不存在的条目 / 不存在的项目 → no-op 不报错
        store.remove_trust("/proj", "a").expect("重复删除应 no-op");
        store
            .remove_trust("/never-existed", "x")
            .expect("未知项目应 no-op");
        let _ = std::fs::remove_dir_all(dir);
    }
}
