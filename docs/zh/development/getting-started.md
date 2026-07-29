# 开发环境

## 必需依赖

- Git；
- Node.js 20.19 或更高版本与 npm；
- Rust 1.97.1（仓库校验脚本使用该锁定工具链，并要求安装 `rustfmt`、`clippy`）；
- 当前平台的 [Tauri 2 prerequisites](https://v2.tauri.app/start/prerequisites/)；
- 与 AIO 同级的 `aio-models-data` 仓库。

`package.json` 使用 `file:../aio-models-data`，且打包时需要其 `dist/data/models.json`：

```text
workspace/
├── AIO/
└── aio-models-data/
    └── dist/data/models.json
```

## 获取代码

```bash
git clone https://github.com/Atom112/aio-models-data.git
git clone https://github.com/Atom112/AIO.git
cd AIO
npm ci
```

## 开发与检查

```bash
# 前端开发服务器
npm run dev

# Tauri 桌面开发
npm run tauri dev

# 前端生产构建
npm run build

# 提交前完整校验
npm run verify
```

`npm run dev` 只启动 Vite，无法验证 Tauri invoke、文件选择、系统凭据库、本地引擎或更新功能。日常功能开发应使用 `npm run tauri dev`。

仓库的 `.cargo/config.toml` 将 crates.io 替换为 rsproxy 稀疏镜像，用于规避部分 Windows 网络中 IPv6 + Schannel 的 TLS 握手失败。依赖版本和校验和仍由 `Cargo.lock` 约束；若所在组织要求直接访问官方源，可删除本地 source replacement 后构建。

## Linux

发布工作流在 Ubuntu 22.04 安装：

```bash
sudo apt-get update
sudo apt-get install -y \
  libwebkit2gtk-4.1-dev \
  libappindicator3-dev \
  librsvg2-dev \
  patchelf \
  rpm
```

其他发行版请使用 Tauri 文档给出的等价包名。

## 发布构建

```bash
npm run tauri build
```

Tauri 配置声明 `deb`、`rpm`、`nsis`、`msi`、`app` 和 `dmg` 目标。实际发布由 `.github/workflows/release.yml` 在 Windows、Ubuntu 和两个 macOS 架构上串行构建，并生成更新元数据。

本地构建不会自动获得发布签名密钥。缺少签名环境变量时，不要用本地包替代正式 Releases 产物。

## 提交前

统一运行：

```bash
npm run verify
```

`verify` 会依次检查 i18n 字典、文档链接、Prettier、ESLint、TypeScript、前端生产构建、Rustfmt、Clippy 严格模式和 Rust 测试。需要单独定位问题时，可运行 `npm run format:check`、`npm run lint`、`npm run typecheck`、`npm run check:frontend` 或 `npm run check:rust`。

新增 Rust Tauri command 时还要确认模块导出、`lib.rs` 注册和命令文档注释。新增界面样式使用 Tailwind 工具类，代码文本中不要加入 Emoji。
