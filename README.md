# CCSwitch

Claude Code 模型配置管理器 — Rust TUI + CLI 工具。

管理 Claude Code 与 Codex 的多个 API 供应商配置，支持一键切换、代理转发、会话历史浏览。Claude Code 支持本地/代理模式，Codex 直接维护自身配置文件。

## 功能

- **Claude 四模型配置**：每个 Profile 独立配置 Opus、Sonnet、Haiku、Subagent
- **双应用顶栏**：顶栏显示 `Claude | Codex`，Space 切换当前应用上下文
- **Codex Provider 管理**：Codex 只配置 Provider，无需 Profile，删除时同步清理 Codex config.toml
- **一键切换**：本地模式直接写入 `~/.claude/settings.json`，代理模式更新 SQLite
- **会话历史**：分别扫描 Claude Code 与 Codex 本地会话文件，支持搜索、过滤
- **代理服务**：本地 HTTP 代理（端口 15721），自动模型名转换，支持 systemd / launchd / 计划任务后台运行
- **Token 用量**：按模型统计 Token 消耗，带缓存避免每帧查询
- **多语言**：内置中文 / English 切换，设置持久化

## 安装

### NixOS

#### Home Manager（推荐，按用户配置）

在 `home.nix` 中：

```nix
{
  inputs.ccswitch.url = "github:your/ccswitch";

  homeConfigurations = {
    your-user = home-manager.lib.homeManagerConfiguration {
      modules = [
        ccswitch.homeModules.default
        {
          programs.ccswitch = {
            enable = true;
            # 由 sops-nix/agenix 等部署到 Nix store 外；内容如 OPENAI_API_KEY=sk-xxx
            envVars = "%h/.config/ccswitch/env";
            defaults = {
              version = 1;
              providers = [
                {
                  id = "deepseek";
                  name = "DeepSeek";
                  api_url = "https://api.deepseek.com/anthropic";
                  api_key = "env:DEEPSEEK_API_KEY";
                  profiles = [
                    {
                      id = "v4"; name = "V4";
                      opus = "deepseek-v4-pro[1m]";
                      sonnet = "deepseek-v4-pro[1m]";
                      haiku = "deepseek-v4-flash";
                      subagent = "deepseek-v4-flash";
                      default = true;
                    }
                  ];
                }
              ];
              codex_providers = [
                {
                  id = "openai-api";
                  name = "OpenAI API";
                  api_url = "https://api.openai.com/v1";
                  api_key = "env:OPENAI_API_KEY";
                }
              ];
            };
          };
        }
      ];
    };
  };
}
```

Home Manager 会自动：

- 将 `defaults` 写入 `~/.config/ccswitch/defaults.toml`
- 安装并启用 `ccs-proxy` systemd user service（通过 `envVars` 传入 Nix store 外的环境文件路径）

注意：`defaults` 会进入 Nix store，`api_key` 应只写 `env:VAR_NAME` 引用，不要写明文密钥。

#### NixOS 全局安装

在 `configuration.nix` 中：

```nix
{
  inputs.ccswitch.url = "github:your/ccswitch";

  outputs = { nixpkgs, ccswitch, ... }: {
    nixosConfigurations.your-host = nixpkgs.lib.nixosSystem {
      modules = [
        ccswitch.nixosModules.default
        {
          services.ccswitch = {
            enable = true;
            defaults = {
              version = 1;
              providers = [ ... ];
              codex_providers = [
                {
                  id = "openai-api";
                  name = "OpenAI API";
                  api_url = "https://api.openai.com/v1";
                  api_key = "env:OPENAI_API_KEY";
                }
              ];
            };
          };
        }
      ];
    };
  };
}
```

NixOS 模块将配置写入 `/etc/ccswitch/defaults.toml`，并安装二进制包。

#### 命令行直接使用

```bash
# 临时启动
nix run github:your/ccswitch

# 安装到 profile
nix profile install github:your/ccswitch
```

### Homebrew（macOS）

```bash
brew tap pomeluce/ccswitch
brew install ccswitch
```

### Cargo（Linux / macOS / Windows WSL2）

```bash
cargo install --git https://github.com/pomeluce/ccswitch
```

首次运行会自动创建 `~/.config/ccswitch/` 目录（包含数据库和配置文件）。

### 预编译包（Linux）

从 [Releases](https://github.com/pomeluce/ccswitch/releases) 下载：

```bash
# Debian/Ubuntu
curl -LO https://github.com/pomeluce/ccswitch/releases/latest/download/ccs_<version>_linux-x86_64.deb
sudo dpkg -i ccs_*.deb

# Fedora/RHEL
curl -LO https://github.com/pomeluce/ccswitch/releases/latest/download/ccswitch-<version>.x86_64.rpm
sudo rpm -i ccswitch-*.rpm

# 通用 tar.gz
curl -LO https://github.com/pomeluce/ccswitch/releases/latest/download/ccs_<version>_linux-x86_64.tar.gz
tar -xzf ccs_*.tar.gz
sudo mv ccs /usr/local/bin/
```

### 预编译包（macOS 手动）

```bash
curl -LO https://github.com/pomeluce/ccswitch/releases/latest/download/ccs_<version>_darwin-arm64.tar.gz
tar -xzf ccs_*.tar.gz
chmod +x ccs
sudo mv ccs /usr/local/bin/
```

### 预编译包（Windows）

从 [Releases](https://github.com/pomeluce/ccswitch/releases) 下载 zip 或 exe，解压后将 `ccs.exe` 放到 `%PATH%` 中。

也可以通过 **Scoop** 安装：

```bash
scoop bucket add ccswitch https://github.com/pomeluce/scoop-ccswitch.git
scoop install ccswitch
```

安装后执行 `ccs service install` 注册 Windows 计划任务后台服务。

## 使用

### TUI 模式

```bash
ccs    # 无参数启动 TUI
```

左侧栏 `J/K` 选择标签页，`Tab` / `Shift+Tab` 切换，`Enter` 确认。

### CLI 模式

```bash
# 模型切换
ccs switch deepseek/v4           # 切换到指定的 provider/profile
ccs list                         # 列出所有 provider 和 profile

# 配置管理（仅对用户配置生效，系统默认不可删除/编辑）
ccs add provider                 # 交互式添加供应商
ccs add profile <provider-id>    # 添加模型配置
ccs edit <provider|profile>      # 查看配置
ccs remove <provider|profile>    # 删除用户配置

# 代理服务
ccs proxy start                  # 后台启动代理（自动检测 systemd）
ccs proxy stop                   # 停止代理
ccs proxy status                 # 查看代理状态
ccs proxy serve                  # 前台运行代理（调试用）
ccs service install              # 安装后台服务（开机自启）
ccs service uninstall            # 卸载后台服务

# 服务安装平台支持：
#   Linux   → systemd user service
#   macOS   → launchd agent
#   Windows → 计划任务 (Schtasks)

# 用量与历史
ccs usage                        # Token 用量统计（默认本周）
ccs usage --day|--week|--month   # 按日/周/月
ccs usage --profile <name>       # 按模型过滤
ccs history                      # 会话历史
ccs history --project <name>     # 按项目过滤
ccs history --search <keyword>   # 搜索会话

# Shell 补全 & Man 文档
ccs completions <zsh|bash|fish>  # 生成 Shell 补全脚本
ccs man                          # 输出 roff 格式 man page
```

## 配置

配置文件位置：

- `~/.config/ccswitch/ccswitch.db` — SQLite 数据库（模型配置、用量、会话）
- `~/.config/ccswitch/defaults.toml` — 系统默认配置（Home Manager / NixOS 生成）
- `~/.local/share/ccswitch/ccs.log` — TUI 运行日志
- `~/.codex/config.toml` — Codex Provider 配置
- `~/.codex/auth.json` — Codex API Key

### 首次启动

首次启动 `ccs` 时会先显示终端进度条导入 Claude Code 历史会话数据（从 `~/.claude/projects/` 扫描 JSONL 文件）。导入完成后自动进入 TUI。后续启动跳过导入直接进入。

用量数据在进入 TUI 后通过后台异步扫描。Claude 数据来自 `~/.claude/projects`，Codex 数据来自 `~/.codex/sessions`；会话与用量面板约每秒检测变化并实时刷新。Codex rename 后的会话标题从 `~/.codex/session_index.jsonl` 同步。

### defaults.toml

```toml
version = 1

[[providers]]
id = "deepseek"
name = "DeepSeek"
api_url = "https://api.deepseek.com/anthropic"
api_key = "env:DEEPSEEK_API_KEY"

[[providers.profiles]]
id = "v4"
name = "V4"
opus = "deepseek-v4-pro[1m]"
sonnet = "deepseek-v4-pro[1m]"
haiku = "deepseek-v4-flash"
subagent = "deepseek-v4-flash"
default = true

[[providers]]
id = "openrouter"
name = "OpenRouter"
api_url = "https://openrouter.ai/api"
api_key = "env:OPENROUTER_API_KEY"

[[providers.profiles]]
id = "claude"
name = "Claude"
opus = "anthropic/claude-opus-4"
sonnet = "anthropic/claude-sonnet-4"
haiku = "anthropic/claude-haiku-4"
subagent = "anthropic/claude-haiku-4"

[[codex_providers]]
id = "codex-proxy"
name = "Codex Proxy"
api_url = "https://api.example.com/v1"
api_key = "env:OPENAI_API_KEY"
```

### API Key 格式

| 格式           | 说明                                             |
| -------------- | ------------------------------------------------ |
| `env:VAR_NAME` | 从进程环境或 `~/.config/ccswitch/env` 读取，推荐 |
| `sk-xxx...`    | 直接文本（明文存储，不安全）                     |
| 空值           | Claude 使用 `$CLAUDE_API_KEY`；Codex 使用 `$OPENAI_API_KEY` |

## TUI 快捷键

### 全局

| 键                  | 功能             |
| ------------------- | ---------------- |
| `Tab` / `Shift+Tab` | 切换侧边栏标签页 |
| `Space`             | 切换顶栏 `Claude | Codex` Tab |
| `Q` / `q`           | 退出             |

### 模型标签页

Claude Code 使用两层导航：左侧 Provider 列表 → 右侧 Profile 列表。Codex 只显示 Provider 列表，`Enter` 直接切换。

| 键          | 功能                              |
| ----------- | --------------------------------- |
| `J/K` `↑/↓` | Provider 列表导航 / Profile 导航  |
| `Enter`     | 进入 Profile 列表 / 切换模型      |
| `A`         | 添加 Provider / Profile           |
| `E`         | 编辑 Provider / Profile           |
| `D`         | 删除 Provider / Profile（需确认） |
| `Esc`       | 返回 Provider 列表                |

### 会话标签页

| 键          | 功能                                          |
| ----------- | --------------------------------------------- |
| `J/K` `↑/↓` | 上下导航（循环滚动）                          |
| `Enter`     | 打开会话（根据顶栏启动 Claude Code 或 Codex） |
| `D`         | 删除会话（弹窗确认）                          |
| `/`         | 搜索（分词匹配标题 + 项目名）                 |
| `Esc`       | 退出搜索 / 关闭弹窗                           |

### 用量标签页

| 键            | 功能                     |
| ------------- | ------------------------ |
| `J/K` `↑/↓`   | 导航模型列表             |
| `T`           | 切换时间范围（天/周/月） |
| `/`           | 搜索模型                 |
| `PgUp`/`PgDn` | 滚动右侧日用量图表       |

左侧显示选中模型的今日/本周/总计/请求数统计卡片及模型排名。右侧显示选中模型的近 7 天用量柱状图。首次启动时用量数据在后台异步扫描，右侧面板显示扫描进度条。

### 设置标签页

单面板居中布局，所有设置项垂直排列。

| 键          | 功能       |
| ----------- | ---------- |
| `J/K` `↑/↓` | 选择设置项 |
| `H/L` `←/→` | 切换选项值 |

设置面板由 Claude 与 Codex 共用，支持切换主题（7 种）、模式（local / proxy）、语言（中文 / English）。模式设置只对 Claude 生效；Codex 不区分 local/proxy。

## 写入映射

切换 profile 时写入 Claude Code 的 `settings.json`。

### Local 模式

| 环境变量                         | 值                      |
| -------------------------------- | ----------------------- |
| `ANTHROPIC_AUTH_TOKEN`           | 解析后的 API key        |
| `ANTHROPIC_BASE_URL`             | 上游 API 地址           |
| `ANTHROPIC_MODEL`                | `sonnet`                |
| `ANTHROPIC_DEFAULT_OPUS_MODEL`   | `opus`                  |
| `ANTHROPIC_DEFAULT_SONNET_MODEL` | `sonnet`                |
| `ANTHROPIC_DEFAULT_HAIKU_MODEL`  | `haiku`（去 `[1m]`）    |
| `CLAUDE_CODE_SUBAGENT_MODEL`     | `subagent`（去 `[1m]`） |

### Proxy 模式

| 环境变量               | 值                         |
| ---------------------- | -------------------------- |
| `ANTHROPIC_AUTH_TOKEN` | `ccswitch-proxy`（占位符） |
| `ANTHROPIC_BASE_URL`   | `http://127.0.0.1:15721`   |

Opus/Sonnet/Haiku 变量不在 settings.json 中设置，由 proxy server 透明处理；Subagent 使用内部标记以便代理识别并映射。

### Codex Provider 切换

Codex 不使用 Profile，也不区分 local/proxy。切换 Provider 时保留配置文件中的其他设置和已有 Provider，并写入：

```toml
model_provider = "codex-proxy"

[model_providers.codex-proxy]
name = "Codex Proxy"
base_url = "https://api.example.com/v1"
wire_api = "responses"
requires_openai_auth = true

[ccswitch.last_switch]
source = "codex-proxy"
at = "2026-01-01 12:00:00"
```

同时更新 `~/.codex/auth.json`：

```json
{
  "OPENAI_API_KEY": "解析后的 provider API key"
}
```

## 模式

### 本地模式（Local）

直接修改 `~/.claude/settings.json` 的 `env` 字段，Claude Code 直接访问上游 API。

### 代理模式（Proxy）

1. `ccs service install` 安装后台服务（或 `ccs proxy start` 手动启动）
2. 代理监听 `127.0.0.1:15721`
3. `settings.json` 的 `ANTHROPIC_BASE_URL` 指向代理，Claude Code 所有请求经过代理
4. 代理自动进行模型名转换：
   - 请求体：Opus/Sonnet/Haiku/Subagent 分别映射到对应 Profile 字段，并去除 `[1m]`
   - 响应流：`message.model` 还原为原始名称，注入 `ccs_model` / `ccs_proxy` 标记
5. 切换 profile 无需重启代理，代理每次请求从 DB 读取最新配置
6. 自动记录 Token 用量到 SQLite

## 开发

```bash
nix develop    # 进入开发环境（Rust 工具链）
cargo build    # 构建
cargo test     # 测试
cargo run      # 启动 TUI
nix build .#   # Nix 打包
```

## License

GPL-3.0
