# maa-auto-reverse-rs

`maa-auto-reverse-rs` 是 `MaaAutoReverse` 的 Rust 重构版，面向 Windows 平台，基于 `MAA Framework`、`iced` 和 `RON` 配置实现。

当前版本以 GUI 为主要使用方式，同时保留一个用于调试的命令行单次识别入口。

## 功能特性

- 使用 `iced` 提供桌面 GUI
- 使用 `maa-framework` Rust 绑定执行自动化
- 使用 `RON` 持久化应用设置、策略配置和预设数据
- 支持 `F8` 启动/停止自动倒转
- 支持 `F9` 启动/停止刷新保留模式
- 支持单次扫描识别与调试图像预览
- 支持导出识别结果中的原图、标注图、命中图和各槽位 ROI 图
- 支持从旧版 JSON 配置自动导入
- 支持 `pipeline override` 启动加载与热重载

## 运行要求

- Windows
- 管理员权限
- 项目目录下存在可用的 `runtime/` 和 `resource/` 资源

程序启动时会自动请求管理员权限。  
默认按项目根目录下的以下结构查找运行资源：

```text
config/
data/
resource/
runtime/
src/
Cargo.toml
```

## 快速开始

### 1. 开发环境运行 GUI

```powershell
cargo run -- gui
```

### 2. 命令行执行一次扫描

```powershell
cargo run -- scan-once --window 明日方舟
```

如果不传 `--window`，会回退到当前保存的窗口标题。

## GUI 说明

GUI 提供这些核心能力：

- 选择和刷新目标窗口
- 切换 `90% / 100%` 界面比例
- 启动自动倒转
- 启动刷新保留模式
- 编辑四类名单
- 勾选预设道具和预设保留干员
- 查看日志
- 查看单次扫描的识别结果和调试图像
- 通过系统保存对话框导出调试图片

运行中还支持：

- `F8` 切换自动倒转
- `F9` 切换刷新保留
- 手动按 `D` 触发立即重扫

## 配置文件

程序主要使用以下 RON 文件：

- `data/app_settings.ron`
- `data/strategy_config.ron`
- `data/presets.ron`

首次启动时，如果还没有这些 RON 文件，会尝试从旧版配置导入，包括：

- `config/maa_option.json`
- `config/advanced_config.json`
- `config/buy_items.json`
- `config/buy_sell_operators.json`
- `config/buy_only_operators.json`
- `config/six_star_operators.json`
- `config/predefined_items.json`
- `config/predefined_buy_only_operators.json`

## Pipeline Override

如果项目根目录存在：

```text
config/pipeline_override.json
```

程序会在启动任务时自动加载它，并在运行中监听文件变化，检测到修改后自动热重载。

日志中会明确提示：

- 是否检测到 override 文件
- 是否启用了热重载
- 是否重载成功

## 日志

日志默认写入：

```text
logs/maa_auto_reverse_YYYYMMDD.log
```

GUI 中也会同步显示最近日志。

## 项目结构

```text
src/
  app.rs                 GUI
  main.rs                CLI 入口
  domain/                策略、图像处理、引擎
  infra/                 MAA、窗口、热键、持久化、Win32 输入
  orchestrator/          运行状态和协调层
config/                  旧版配置与 override 文件
data/                    RON 配置
resource/                MAA bundle 资源
runtime/                 MAA 运行时二进制
.github/workflows/       CI/CD 工作流
```

## 构建发布

仓库内置了 GitHub Actions 工作流：

```text
.github/workflows/build-windows-release.yml
```

行为如下：

- 推送 `v` 开头标签时自动构建 Windows 可执行文件
- 将二进制与 `config/`、`data/`、`resource/`、`runtime/` 一起打包
- 上传为 Actions artifact
- 自动发布 GitHub Release
- 如果标签名包含 `-`，则发布为 `prerelease`
- 支持手动触发工作流，但手动触发不会发布 Release

## 本地构建

```powershell
cargo build --release
```

构建产物默认位于：

```text
target/release/maa-auto-reverse-rs.exe
```

## 说明

- 当前版本首要目标平台是 Windows
- 当前版本只实现 Win32 控制链路，不包含 ADB 交付
- GUI 不是对旧版 Tk 界面的逐像素复刻，而是等价能力的 Rust 桌面实现
