# Install Nix packages from immutable release assets

AkironMux 的 Nix packages、NixOS module 和 Home Manager module 使用 GitHub Release 中固定 SHA-256 的预编译 CLI tarball 与 desktop deb，而不再通过下游 nixpkgs、pnpm 和 Rust 工具链从源码构建。Release 采用 prepare/publish 两阶段流程：先构建并保存候选资产、提交其 Nix hash，再为该提交创建 Tag 并发布完全相同的文件；同一 Tag 的资产不得覆盖，以保持固定 URL 的内容稳定。

## Consequences

开发环境和 Release CI 仍从源码构建。由于当前 Release 只生成 Linux x86_64 资产，Nix release packages 暂时只支持 `x86_64-linux`；其他架构必须先加入对应的 Release 资产和 hash，不能回退为源码安装。
