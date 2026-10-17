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

---

# 修复记录（2026-10-17，修复轮）

由修复代理对全部五项逐条复核、实测并修复。**先修正一项关键结论**：

## 第 1 项根因修正：不是「本机 junction 遍历整体失效」，而是「受限进程的 reparse 穿越被 Windows 按设计拒绝」

原报告的探针全部在 AIO 自身的 agent shell（默认 `sandboxWindows=appcontainer`）内执行。**AppContainer / 受限令牌进程穿越 junction/mount point 时，除非目标 ACL 显式授予该容器 SID，否则 Windows 按安全设计拒绝**（`cmd` 的「不受信任的装入点」错误正是这一层的标准文案）。`whoami /groups` 显示 Medium 完整性、无 AppContainer SID 并不能排除——观察到的失败签名与 AppContainer reparse 策略完全一致。

**决定性反证**：同一时刻、同一机器、同一用户，从普通 Medium-IL 进程（另一个 harness 的 pwsh 会话）对同一个 junction 执行 `lstat / readdir / readFileSync` 全部成功。自建 junction 在 `%TEMP%` 与 `D:\base` 的失败同理——不是文件系统损坏，是发起方身份。

### 修复（已落地）

`.npmrc` 增加 `install-links=true`：npm 对 `file:` 依赖装成**真实目录副本**而非 junction——副本位于项目内的 `node_modules/`，对宿主会话与受限会话（AIO 自迭代沙箱）**同样可读**。实测：

- `npm ci`（7s）后 `lstatSync(...).isSymbolicLink() === false`，`dist/data/models.json` 在位
- `npx tsc --noEmit` 通过（不再有 5 个 TS2307）
- `cargo check` / `test` 无需 `TAURI_CONFIG` 覆盖即可通过（`bundle.resources` 解析到真实副本）
- `vite build` 通过

**AIO 自迭代的全部绕过手段（TAURI_CONFIG 覆盖、间接验证 tsc）自此不再需要**——在它自己的沙箱里直接跑全量门禁即可。

### 遗留 → 已完成（次日提权执行）

`fltmc filters` 已跑（输出存 `%TEMP%\aio-elev-diag.txt`）：标准系统筛选器（bindflt/UCPD/WdFilter/storqosflt/wcifs/CldFlt/bfs/FileCrypt/luafv/UnionFS/npsvctrig/Wof/FileInfo）之外，存在两支**非系统过滤驱动 `ACE-CORE202706` / `ACE-CORE102706`**（高度 366667.x、9 实例）——本机第三方安全软件的文件系统过滤层，即「EDR 对安装目录的额外过滤」系列实录（python 的 D:\Anaconda、git 的 D:\Git）的载体。AppLocker（AppIDSvc Stopped、无策略、无事件）、Defender ASR（无规则、无 1121/1122 阻断事件）、WDAC/CodeIntegrity（无 git 相关阻断事件）全部排除。

## 第 2 项根因确认：DACL 拒写 → 授权从未生效 → 静默拒绝

逐层实测定位（全部在普通会话复现）：

1. AppContainer 家族测试里，System32 出身的 `cmd` / `powershell` 全部正常；**只有 `D:\Git\cmd\git.exe` 静默拒绝**——「安装目录被过滤」成立
2. 直接复现：`icacls D:\Git\cmd /grant ...` → **Access is denied**（该目录 DACL 全部为继承项、属主 Administrators；中完整性用户无 WRITE_DAC）
3. 测试内 `grant_dir_access(...)` 返回 `SetNamedSecurityInfoW` 失败（Win32 错误 5），但结果被 `let _ =` 吞掉——授权从未写入，断言随后的静默拒绝必然触发

即：**非产品回归——当时定位到的是「授权写不进 DACL」这一步的确定性失败**。0.8.2 发版前后该测试时绿时红，是因为 `D:\Git` 的授权状态被安全层周期性改写（0.8.1 开发期已有同款实录，测试注释里的 EDR 提示即此）。（次日提权复检后结论再精化一层：把 DACL 修好后 in-AC exec 仍被静默拦截——见下方「遗留」段的过滤驱动证据，「授权写不进」只是第一层，「exec 被过滤」是第二层，两层叠加构成完整根因。）

### 修复（已落地，次日按实测精化为「先运行、后判定」）

测试改为**先运行、后判定**：授权结果不再被 `let _ =` 吞掉；沙箱内 git leg 先跑，仅当「尽力授权失败 **且** 运行为静默拒绝（无输出无报错——本测试要抓的签名）」双条件成立时才跳过（`[SKIP]` 前缀 + 根因与复检指引）；运行成功（预置 AAP ACE 等环境路径）或 git 响亮失败均照走原硬断言，其余断言（.git 拒写、.git 可读、项目根可写）全程硬断言。与 `ac_python_forwarding` 的机器级过滤 `#[ignore]` 先例同构，但本例是运行时判定而非静态忽略。

### 遗留 → 已执行 + 结论修正（次日提权）

已提权执行 `icacls D:\Git /grant "*S-1-15-2-1:(OI)(CI)(RX)" "*S-1-15-2-2:(OI)(CI)(RX)" /T /C`——ACE 全树生效（root 显式、cmd/mingw64 继承，icacls 回读确认）。**但 in-AC `git log` 仍为静默拒绝**（exit 1、零输出）——结合上面 fltmc 的证据：安装目录的 in-AC 执行被 ACE-CORE 过滤驱动拦截（读放行、exec 否决，与 DACL 无关），这就是完整根因的第二层。**机器侧唯一根治路径**：在该安全软件的管理控制台放行（用户侧动作，仓库无能为力）；仓库侧的正确行为即上述确定性跳过。AAP 授予保留——在没有这类过滤的机器上，它就是生产沙箱内 git 可用的那一行修复。

## 第 3 项修复（已落地）：UTF-8 控制台前导

解码器本就是 UTF-8 优先（`decode_console_output`），乱码的真实机制是**混合编码流**：cmd/PowerShell 以 GBK 写系统消息、node/cargo 以 UTF-8 写，合流后整体 UTF-8 校验失败 → 整流按 GBK 解码 → UTF-8 段变成「鏋舵瀯闂ㄧ鍚」。修复在**源头**统一编码：

- PowerShell 路径（`pwsh_script`）：前导 `try { [Console]::OutputEncoding=[System.Text.UTF8Encoding]::new($false); $OutputEncoding=[Console]::OutputEncoding } catch { }`——本会话输出与转发的原生子进程输出均为 UTF-8
- cmd 路径（原生 + AppContainer 两处）：`chcp 65001 >nul & <命令>`——隐藏控制台切换到 65001，cmd 内建消息与感知控制台的子进程随之 UTF-8
- 前导在元字符/危险命令扫描**之后**注入，不触发拦截；`decode_console_output` 的 GBK 回退保留（直接 spawn 的 git 等仍可能按 ACP 输出）
- 既有 e2e（`gbk_console_output_decodes_e2e`）继续覆盖端到端链路（现走 UTF-8 路径）

## 第 4 项修复（已落地）：分支拓扑复位

三项迭代提交已从 `master` 挪到 `dev-<功能>` 分支并走 PR 合入 `dev`；本地 `master` 复位到 `origin/master`（v0.8.2 发布点）。`master` 重新只含已发布内容。

## 第 5 项：未触碰

`stash@{0}: On dev: aio-checkpoint` 原样保留。

---

## 迭代成果审查结论（修复代理）

三项提交逐行审查通过：TTFT 后端全 delta 分支首 token 打点 + v19 落库迁移 + 前端读出，设计完整（时钟覆盖网关排队与 prefill）；控制台闪窗根因（release GUI 子系统无控制台 + 管道化不阻止控制台分配）分析正确，`HiddenConsole` trait 统一 25+ 处 spawn 并加门禁防回归；`is_git_repo` 10s TTL 记忆化 + git init/clone 后失效，消除每回合重复 spawn。**发现并修复其一项测试盲点**（第 2 项的授权吞错）。
