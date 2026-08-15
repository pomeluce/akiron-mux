{
  description = "AkironMux — unified Claude Code and Codex configuration manager";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-parts.url = "github:hercules-ci/flake-parts";
    rust-overlay.url = "github:oxalica/rust-overlay";
  };

  outputs =
    {
      self,
      nixpkgs,
      flake-parts,
      rust-overlay,
      ...
    }@inputs:
    flake-parts.lib.mkFlake { inherit inputs; } {
      systems = [
        "x86_64-linux"
        "aarch64-linux"
      ];

      perSystem =
        { system, ... }:
        let
          overlays = [ (import rust-overlay) ];
          pkgs = import nixpkgs { inherit system overlays; };
          rust = pkgs.rust-bin.stable.latest.default;
          rustWindows = pkgs.rust-bin.stable.latest.default.override {
            targets = [ "x86_64-pc-windows-msvc" ];
          };
          rustPlatform = pkgs.makeRustPlatform {
            cargo = rust;
            rustc = rust;
          };
          version = "1.11.0";
          tuiPackage = rustPlatform.buildRustPackage {
            pname = "akiron-mux";
            inherit version;
            src = ./.;
            pnpmRoot = "web/session-ui";
            pnpmDeps = pkgs.fetchPnpmDeps {
              pname = "akiron-mux-webui";
              inherit version;
              src = ./web/session-ui;
              pnpm = pkgs.pnpm_11;
              fetcherVersion = 4;
              hash = "sha256-0waT+Kvwk5CB8+LJcps4EdZZn0EZsHLFJu2D3MOHF1A=";
            };
            cargoLock.lockFile = ./Cargo.lock;
            nativeBuildInputs = [
              pkgs.installShellFiles
              pkgs.nodejs
              pkgs.pnpmConfigHook
              pkgs.pnpm_11
            ];
            preBuild = ''
              (cd web/session-ui && pnpm build)
            '';
            postInstall = ''
              installShellCompletion --zsh --name _akmux \
                <($out/bin/akmux completions zsh)
              installShellCompletion --bash --cmd akmux \
                <($out/bin/akmux completions bash)
              installShellCompletion --fish --cmd akmux \
                <($out/bin/akmux completions fish)
              installManPage --name akmux.1 <($out/bin/akmux man)
            '';
            meta.mainProgram = "akmux";
          };
          desktopPackage = rustPlatform.buildRustPackage {
            pname = "akiron-mux-desktop";
            inherit version;
            src = ./web/session-ui;
            cargoRoot = "src-tauri";
            buildAndTestSubdir = "src-tauri";
            cargoLock.lockFile = ./web/session-ui/src-tauri/Cargo.lock;
            pnpmDeps = pkgs.fetchPnpmDeps {
              pname = "akiron-mux-webui";
              inherit version;
              src = ./web/session-ui;
              pnpm = pkgs.pnpm_11;
              fetcherVersion = 4;
              hash = "sha256-0waT+Kvwk5CB8+LJcps4EdZZn0EZsHLFJu2D3MOHF1A=";
            };
            nativeBuildInputs = [
              pkgs.nodejs
              pkgs.pkg-config
              pkgs.pnpmConfigHook
              pkgs.pnpm_11
              pkgs.wrapGAppsHook3
            ];
            buildInputs = [
              pkgs.glib-networking
              pkgs.libsoup_3
              pkgs.openssl
              pkgs.webkitgtk_4_1
            ];
            preBuild = ''
              pnpm build
            '';
            postInstall = ''
              install -Dm644 public/akiron.svg \
                $out/share/icons/hicolor/scalable/apps/akiron-mux.svg
              install -Dm644 ${./assets/akiron-mux.desktop} \
                $out/share/applications/akiron-mux.desktop
            '';
            meta.mainProgram = "akiron-mux";
          };
          guiPackage = pkgs.symlinkJoin {
            name = "akiron-mux-${version}-with-gui";
            paths = [
              tuiPackage
              desktopPackage
            ];
            meta.mainProgram = "akmux";
          };
        in
        {
          packages = {
            default = tuiPackage;
            tui = tuiPackage;
            desktop = desktopPackage;
            gui = guiPackage;
          };

          devShells.default = pkgs.mkShell {
            name = "akiron-mux-dev";
            buildInputs = [
              rust
              pkgs.cargo
              pkgs.rust-analyzer
              pkgs.clippy
              pkgs.rustfmt
              pkgs.pkg-config
            ];
            shellHook = ''
              echo "AkironMux dev shell"
              echo "  cargo build   — build"
              echo "  cargo test    — run tests"
              echo "  cargo run --bin akmux — launch TUI"
              echo "  nix build .#  — build package"
            '';
          };

          devShells.gui = pkgs.mkShell {
            name = "akironmux-gui-dev";
            buildInputs = [
              rustWindows
              pkgs.cargo
              pkgs.cargo-tauri
              pkgs.cargo-xwin
              pkgs.nodejs
              pkgs.pnpm_11
              pkgs.pkg-config
              pkgs.glib
              pkgs.gtk3
              pkgs.webkitgtk_4_1
              pkgs.libsoup_3
              pkgs.cairo
              pkgs.pango
              pkgs.gdk-pixbuf
              pkgs.atk
              pkgs.openssl
              pkgs.librsvg
              pkgs.dpkg
              pkgs.patchelf
              pkgs.nsis
              pkgs.clang
              pkgs.llvmPackages.llvm
              pkgs.llvmPackages.lld
            ];
            shellHook = ''
              echo "AkironMux GUI dev shell"
              echo "  cd web/session-ui"
              echo "  pnpm desktop:dev"
              echo "  pnpm desktop:build"
            '';
          };
        };

      flake =
        let
          mkDefaultsType =
            lib: pkgs:
            let
              format = pkgs.formats.toml { };
              profileType = lib.types.submodule {
                freeformType = format.type;
                options = {
                  id = lib.mkOption { type = lib.types.str; };
                  name = lib.mkOption { type = lib.types.str; };
                  opus = lib.mkOption {
                    type = lib.types.str;
                    default = "";
                  };
                  sonnet = lib.mkOption {
                    type = lib.types.str;
                    default = "";
                  };
                  haiku = lib.mkOption {
                    type = lib.types.str;
                    default = "";
                  };
                  subagent = lib.mkOption {
                    type = lib.types.str;
                    default = "";
                  };
                  default = lib.mkOption {
                    type = lib.types.bool;
                    default = false;
                  };
                };
              };
              providerType = lib.types.submodule {
                freeformType = format.type;
                options = {
                  id = lib.mkOption { type = lib.types.str; };
                  name = lib.mkOption { type = lib.types.str; };
                  api_url = lib.mkOption { type = lib.types.str; };
                  api_key = lib.mkOption { type = lib.types.str; };
                  profiles = lib.mkOption {
                    type = lib.types.listOf profileType;
                    default = [ ];
                  };
                };
              };
              codexProviderType = lib.types.submodule {
                freeformType = format.type;
                options = {
                  id = lib.mkOption { type = lib.types.str; };
                  name = lib.mkOption { type = lib.types.str; };
                  api_url = lib.mkOption { type = lib.types.str; };
                  api_key = lib.mkOption { type = lib.types.str; };
                  codex_catalog = lib.mkOption {
                    type = lib.types.enum [
                      "built-in"
                      "custom"
                    ];
                    default = "built-in";
                  };
                  models = lib.mkOption {
                    type = lib.types.listOf (
                      lib.types.submodule {
                        freeformType = format.type;
                        options = {
                          slug = lib.mkOption { type = lib.types.str; };
                          display_name = lib.mkOption { type = lib.types.str; };
                          description = lib.mkOption {
                            type = lib.types.str;
                            default = "";
                          };
                          context_window = lib.mkOption {
                            type = lib.types.int;
                            default = 128000;
                          };
                          max_context_window = lib.mkOption {
                            type = lib.types.nullOr lib.types.int;
                            default = null;
                          };
                          effective_context_window_percent = lib.mkOption {
                            type = lib.types.int;
                            default = 95;
                          };
                          default_reasoning_effort = lib.mkOption {
                            type = lib.types.str;
                            default = "medium";
                          };
                          supported_reasoning_efforts = lib.mkOption {
                            type = lib.types.listOf lib.types.str;
                            default = [
                              "low"
                              "medium"
                              "high"
                            ];
                          };
                          input_modalities = lib.mkOption {
                            type = lib.types.listOf lib.types.str;
                            default = [ "text" ];
                          };
                          supports_parallel_tool_calls = lib.mkOption {
                            type = lib.types.bool;
                            default = true;
                          };
                          support_verbosity = lib.mkOption {
                            type = lib.types.bool;
                            default = true;
                          };
                          default_verbosity = lib.mkOption {
                            type = lib.types.str;
                            default = "low";
                          };
                          supports_search_tool = lib.mkOption {
                            type = lib.types.bool;
                            default = false;
                          };
                          default = lib.mkOption {
                            type = lib.types.bool;
                            default = false;
                          };
                        };
                      }
                    );
                    default = [ ];
                  };
                };
              };
            in
            lib.types.submodule {
              options = {
                version = lib.mkOption {
                  type = lib.types.int;
                  default = 1;
                };
                claude_providers = lib.mkOption {
                  type = lib.types.listOf providerType;
                  default = [ ];
                };
                codex_providers = lib.mkOption {
                  type = lib.types.listOf codexProviderType;
                  default = [ ];
                };
              };
            };
        in
        {
          # NixOS system-level module — installs package + generates defaults.toml
          nixosModules.default =
            {
              config,
              lib,
              pkgs,
              ...
            }:
            let
              cfg = config.services.akmux;
              format = pkgs.formats.toml { };
              package =
                if cfg.gui then
                  self.packages.${pkgs.stdenv.hostPlatform.system}.gui
                else
                  self.packages.${pkgs.stdenv.hostPlatform.system}.tui;
            in
            {
              imports = [ (lib.mkRenamedOptionModule [ "services" "ccswitch" ] [ "services" "akmux" ]) ];
              options.services.akmux = {
                enable = lib.mkEnableOption "AkironMux Claude Code and Codex configuration manager";
                gui = lib.mkOption {
                  type = lib.types.bool;
                  default = false;
                  description = "Install the AkironMux desktop GUI in addition to the akmux TUI.";
                };
                defaults = lib.mkOption {
                  type = mkDefaultsType lib pkgs;
                  default = { };
                  description = "Claude and Codex provider configurations (written to /etc/akmux/defaults.toml)";
                };
              };
              config = lib.mkIf cfg.enable {
                environment.systemPackages = [ package ];
                environment.etc."akmux/defaults.toml".source =
                  format.generate "akmux-system-defaults.toml" cfg.defaults;
              };
            };

          # Home Manager user-level module
          homeModules.default =
            {
              config,
              lib,
              pkgs,
              ...
            }:
            let
              cfg = config.programs.akmux;
              package =
                if cfg.gui then
                  self.packages.${pkgs.stdenv.hostPlatform.system}.gui
                else
                  self.packages.${pkgs.stdenv.hostPlatform.system}.tui;
            in
            {
              imports = [ (lib.mkRenamedOptionModule [ "programs" "ccswitch" ] [ "programs" "akmux" ]) ];
              options.programs.akmux = {
                enable = lib.mkEnableOption "AkironMux Claude Code and Codex configuration manager";
                gui = lib.mkOption {
                  type = lib.types.bool;
                  default = false;
                  description = "Install the AkironMux desktop GUI in addition to the akmux TUI.";
                };
                envVars = lib.mkOption {
                  type = lib.types.nullOr lib.types.str;
                  default = null;
                  example = "%h/.config/akmux/env";
                  description = "Path to an environment file kept outside the Nix store for the proxy service.";
                };
                sessionService = {
                  enable = lib.mkOption {
                    type = lib.types.bool;
                    default = true;
                    description = "Restore the AkironMux session backend at login when it is enabled in TUI settings.";
                  };
                  port = lib.mkOption {
                    type = lib.types.port;
                    default = 17321;
                    description = "Loopback port used by the AkironMux session GUI.";
                  };
                };
                defaults = lib.mkOption {
                  type = mkDefaultsType lib pkgs;
                  default = { };
                  description = "Default Claude and Codex provider configurations";
                };
              };
              config = lib.mkIf cfg.enable {
                home.packages = [ package ];

                xdg.configFile."akmux/defaults.toml" =
                  let
                    format = pkgs.formats.toml { };
                  in
                  {
                    source = format.generate "akmux-defaults.toml" cfg.defaults;
                  };

                # Let CLI/TUI processes use the same out-of-store env file as
                # the proxy service without copying secrets into the Nix store.
                xdg.configFile."akmux/env-path" = lib.mkIf (cfg.envVars != null) {
                  text = cfg.envVars;
                };

                # Proxy service
                systemd.user.services.akmux-proxy = {
                  Unit = {
                    Description = "AkironMux Proxy Server";
                    After = [ "network.target" ];
                  };
                  Install = {
                    WantedBy = [ "default.target" ];
                  };
                  Service = {
                    ExecStart = "${package}/bin/akmux proxy serve";
                    Restart = "on-failure";
                    RestartSec = "5";
                  }
                  // lib.optionalAttrs (cfg.envVars != null) {
                    EnvironmentFile = cfg.envVars;
                  };
                };

                systemd.user.services.akmux-sessiond = lib.mkIf cfg.sessionService.enable {
                  Unit = {
                    Description = "AkironMux AI Session Service";
                    After = [ "network.target" ];
                  };
                  Install = {
                    WantedBy = [ "default.target" ];
                  };
                  Service = {
                    ExecStart = "${package}/bin/akmux-sessiond";
                    WorkingDirectory = "%h";
                    Restart = "on-failure";
                    RestartSec = "3";
                    Environment = [ "AKMUX_SESSION_PORT=${toString cfg.sessionService.port}" ];
                  }
                  // lib.optionalAttrs (cfg.envVars != null) {
                    EnvironmentFile = cfg.envVars;
                  };
                };
              };
            };
        };
    };
}
