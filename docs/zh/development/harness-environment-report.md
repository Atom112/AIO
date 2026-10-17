# Harness 环境问题报告

本文记录在 AIO 仓库中接入模型进行自迭代开发时，遇到的**环境层面**问题（不是产品缺陷，也不是代码回归）。目标是让后续代理不必重复踩坑，也便于集中修复。

- 记录时间：2026-10-17
- 环境：Windows（`locx\35719`，Medium 完整性级别，非 AppContainer，非管理员）
- 工具链：Node v26.7.0 / npm 11.19.0 / Rust 1.97.1 / Tauri 2.12

---

## 1.（阻断）`node_modules/@aio/models-data` junction 无法遍历，`tsc` / `cargo check` / `tauri build` 全部失败

### 现象

`package.json` 用 `"@aio/models-data": "file:../aio-models-data"` 引入同级仓库。npm 把它装成 **junction（目录联接）**，而本环境下这个 junction **只能被 lstat 识别，无法被任何进程遍历**。

实测（对仓库自带的 junction）：

| 操作                                             | 结果                                                |
| ------------------------------------------------ | --------------------------------------------------- |
| `fs.lstatSync(...)`                              | 成功，`isSymbolicLink() === true`（识别为重解析点） |
| `fs.statSync(...)`                               | 失败 `UNKNOWN: unknown error, stat`                 |
| `fs.readdirSync('.../dist/data')`                | 失败 `UNKNOWN: unknown error, scandir`              |
| `fs.readFileSync('.../package.json')`            | 失败 `UNKNOWN: unknown error, open`                 |
| `fs.realpathSync(...)`                           | 失败 `UNKNOWN`                                      |
| `cmd /c dir <junction>`                          | **无法遍历该路径，因为它包含不受信任的装入点**      |
| `Get-ChildItem <junction>`                       | 同上（PowerShell 明确报"不受信任的装入点"）         |
| 直读目标目录 `D:\base\aio-models-data\dist\data` | 正常（146 个文件，`models.json` 存在）              |

### 关键结论：不是这一个 junction 的问题，是本机 junction 遍历整体失效

我另外**自建**了两个全新 junction（`mklink /J`，目标都是普通目录，一个在 `%TEMP%`（C 盘）、一个在 `D:\base`），结果完全一致：`lstat` 识别为重解析点，`readdir` / `read` 一律 `UNKNOWN` 失败，`cmd /c dir` 报同样的"不受信任的装入点"。

已排除的可能：

- **不是 AppContainer**：`whoami /groups` 显示 `Mandatory Label\Medium Mandatory Level`（S-1-16-8192），没有任何 AppContainer 包 SID（S-1-15-2-*）。
- **不是 subst / 网络盘**：C、D 均为固定 NTFS 卷（`fsutil fsinfo drivetype`），`Get-PSDrive` 无 `DisplayRoot`。
- **不是符号链接策略**：`fsutil behavior query SymlinkEvaluation` 显示本地到本地"已启用"。
- **不是 Defender 受控文件夹访问**：`EnableControlledFolderAccess = 0`（实时保护为 True）。
- **不是权限不足导致的普通拒绝**：`rmdir <junction>` 可以成功删除，且**目标是完好的**——说明 junction 本身是合法重解析点，只有"穿越解析"这一步被拦。
- **无法确认是否有第三方文件系统过滤驱动**：`fltmc filters` 需要管理员权限，本会话被拒绝（`0x80070005`），这一项留给有提权的代理排查。

补充一个相关限制：本 token **没有 `SeCreateSymbolicLinkPrivilege`**（`whoami /priv` 未列出），且 Developer Mode 未开启（`HKLM\...\AppModelUnlock\AllowDevelopmentWithoutDevLicense` 不存在），所以 `mklink /D`（真符号链接）直接报"你没有足够的权限"。也就是说**无法用符号链接绕过** junction。

### 对开发流程的实际影响

1. **`npx tsc --noEmit` 必然失败**（`npm run typecheck` / `npm run verify` 因此不可用）：

   ```
   src/core/utils/models.ts(7,55): error TS2307: Cannot find module '@aio/models-data' ...
   src/core/utils/models.ts(18,55): error TS2307: ...
   src/features/settings/components/ModelBreakdown.tsx(10,32): error TS2307: ...
   src/features/settings/components/UsageSummaryCards.tsx(9,32): error TS2307: ...
   src/shared/components/ModelRow.tsx(7,32): error TS2307: ...
   ```

   这 5 处是**唯一**的 tsc 错误；改动触及的文件均无类型错误。

2. **`cargo check` / `clippy` / `test` 必然失败**（tauri_build 解析 `bundle.resources`）：

   ```
   resource path `..\node_modules\@aio\models-data\dist\data\models.json` doesn't exist
   ```

3. **`npm run build`（vite）仍能通过** —— 但这是运气：上述 5 处导入**全部是 `import type`**（类型只在编译期使用，打包后被擦除）。只要有人把其中任何一处改成值导入（例如 `import { X }`），前端构建也会立刻断掉。

### 期间使用的绕过方式（供参考，非修复）

```powershell
# Rust 验证：覆盖 bundle.resources，跳过资源解析
cd src-tauri
$env:TAURI_CONFIG='{"bundle":{"resources":[]}}'
cargo +1.97.1 check --quiet
cargo +1.97.1 clippy --all-targets --quiet -- -D warnings
cargo +1.97.1 test --quiet
```

前端只跑了 `vite build` + `vitest`，tsc 通过"确认 5 个错误全部来自该 junction"来间接验证。

### 建议修复方向（交给修复代理判断优先级）

1. **根治（环境侧）**：用提权会话跑 `fltmc filters`，确认是否有安全软件/过滤驱动拦截重解析点穿越；检查 HKCU/HKLM 下与 reparse point、`DisableLocalLinksForPackagedProcesses` 相关的策略。这一项最可能是根因。
2. **规避（安装侧）**：`npm install --install-links`（npm 9+）会把 `file:` 依赖**复制**成真实目录而不是建 junction，可直接绕开。建议先验证该 flag 在本机 npm 11 下的实际行为，再决定是否写进 `README` 的本地开发步骤。
3. **健壮性（仓库侧）**：
   - 前端：给 `@aio/models-data` 在 `tsconfig.json` 里加 `paths` 映射到 `../aio-models-data/dist/bundle/index.d.ts`，这样即使链接失效 tsc 也能解析；
   - 构建：`tauri.conf.json` 的 `bundle.resources` 指向 `../aio-models-data/dist/data/models.json`（同级仓库真实路径）而非穿过 `node_modules`，或在 `build.rs` 里对缺失资源给出可读告警而不是硬失败。

---

## 2.（预存在）`appcontainer_git_history_protection` 单测失败

`cargo test` 结果稳定为 **428 passed / 1 failed / 6 ignored**，失败项固定是：

```
utils::sandbox::win_appcontainer::tests::appcontainer_git_history_protection
src\utils\sandbox\win_appcontainer.rs:1573:
  git log 应给出确定性结果而非 ACL 静默拒绝 (code=Some(1), out=, err=)
```

已确认**与本项目改动无关**：把工作区改动 stash 到干净基线后，该测试同样失败；在 `7322ef2` 上单独检出运行也复现。

失败签名正是该测试专门要抓的"ACL 静默拒绝"（`exit code 1`，stdout/stderr 全空）——即 AppContainer 内的 `git log` 既没成功也没有任何错误输出。测试注释本身已写明"EDR 对特定安装目录的额外过滤（本机实录）留给人工复查"，与本报告第 1 项的结论方向一致：本机存在某种文件访问层拦截。

建议：修复代理先解决第 1 项，再复跑本测试；若仍失败，需要提权排查 EDR/过滤驱动对 `git.exe` 所在目录的处理。

---

## 3.（轻微）控制台代码页 936 导致工具输出乱码

`chcp` = **936（GBK）**。node / rust 以 UTF-8 输出中文时在工具输出里被错误解码，例如：

- `架构门禁失败（1 项）` → `鏋舵瀯闂ㄧ鍚?1 椤癸級:`
- `≤173` → `鈮?73`

不影响判定（数字与英文关键词仍可读），但会让中文报错信息无法直接引用。建议 harness 侧统一 `chcp 65001`，或在读取子进程输出时显式按 UTF-8 解码。

---

## 4.（流程）提交落在 `master` 上，与项目分支规范不符

`AGENTS.md` 规定：日常工作从 `dev` 切 `dev-<功能>`，完成后 PR 回 `dev`；`master` 只放已发布内容。本次两条提交（TTFT 功能、Windows 控制台修复）按用户"帮我提交"的指令落在了 **`master`**。

当前分支关系（提交后）：

- `master` 领先 `dev` 15 个提交，`dev` 没有 `master` 缺失的提交。

建议：修复代理或人工把这两个提交挪到 `dev-<功能>` 分支（或直接 `master` → `dev` 回合并），恢复规范的分支拓扑。

---

## 5.（提示）一个预存在的 stash 条目

`git stash list` 中有一条**不是本次工作产生**的条目：

```
stash@{0}: On dev: aio-checkpoint
```

本次工作创建并 `pop` 了自己的临时 stash，未触碰该条目。修复时请勿误删。

---

## 附：本环境可用的验证命令组合

在 junction 问题修复前，能完整跑通的质量门禁如下（缺 `tsc`）：

```powershell
npm run check:i18n
npm run check:docs
npm run check:gates          # 含新增 console-window 门禁
npx prettier . --check
npx eslint . --max-warnings 0
npx vitest run
npm run build                # 依赖 type-only 导入，暂时可通过

cd src-tauri
$env:TAURI_CONFIG='{"bundle":{"resources":[]}}'
cargo +1.97.1 fmt --check
cargo +1.97.1 clippy --all-targets --quiet -- -D warnings
cargo +1.97.1 test --quiet   # 除第 2 项外应全绿
```
