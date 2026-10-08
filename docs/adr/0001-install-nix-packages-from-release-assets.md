# Install Nix packages from immutable release assets

AkironMux 的 Nix packages、NixOS module 和 Home Manager module 使用 GitHub Release 中固定 SHA-256 的预编译 CLI tarball 与 desktop deb，而不再通过下游 nixpkgs、pnpm 和 Rust 工具链从源码构建。同一 Tag 的资产不得覆盖，以保持固定 URL 的内容稳定。原有手动 prepare/publish 发布顺序由 [ADR-0002](0002-automate-tag-releases-and-sync-nix-metadata.md) 调整为 Tag 自动发布、发布后同步 Nix 清单。

## Consequences

开发环境和 Release CI 仍从源码构建。由于当前 Release 只生成 Linux x86_64 资产，Nix release packages 暂时只支持 `x86_64-linux`；其他架构必须先加入对应的 Release 资产和 hash，不能回退为源码安装。
