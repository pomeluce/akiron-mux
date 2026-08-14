{
  description = "CCSwitch — Claude Code and Codex configuration manager";

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
          rustPlatform = pkgs.makeRustPlatform {
            cargo = rust;
            rustc = rust;
          };
        in
        {
          packages.default = rustPlatform.buildRustPackage {
            pname = "ccswitch";
            version = "1.10.2";
            src = ./.;
            cargoLock = {
              lockFile = ./Cargo.lock;
            };
            nativeBuildInputs = [ pkgs.installShellFiles ];
            postInstall = ''
              installShellCompletion --zsh --name _ccs \
                <($out/bin/ccs completions zsh)
              installShellCompletion --bash --cmd ccs \
                <($out/bin/ccs completions bash)
              installShellCompletion --fish --cmd ccs \
                <($out/bin/ccs completions fish)
              installManPage --name ccs.1 <($out/bin/ccs man)
            '';
          };

          devShells.default = pkgs.mkShell {
            name = "ccswitch-dev";
            buildInputs = [
              rust
              pkgs.cargo
              pkgs.rust-analyzer
              pkgs.clippy
              pkgs.rustfmt
              pkgs.pkg-config
            ];
            shellHook = ''
              echo "🔄 CCSwitch dev shell"
              echo "  cargo build   — build"
              echo "  cargo test    — run tests"
              echo "  cargo run     — launch TUI"
              echo "  nix build .#  — build package"
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
              freeformType = format.type;
              options = {
                version = lib.mkOption {
                  type = lib.types.int;
                  default = 1;
                };
                providers = lib.mkOption {
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
              cfg = config.services.ccswitch;
              format = pkgs.formats.toml { };
            in
            {
              options.services.ccswitch = {
                enable = lib.mkEnableOption "CCSwitch Claude Code and Codex configuration manager";
                defaults = lib.mkOption {
                  type = mkDefaultsType lib pkgs;
                  default = { };
                  description = "Claude and Codex provider configurations (written to /etc/ccswitch/defaults.toml)";
                };
              };
              config = lib.mkIf cfg.enable {
                environment.systemPackages = [ self.packages.${pkgs.stdenv.hostPlatform.system}.default ];
                environment.etc."ccswitch/defaults.toml".source =
                  format.generate "ccswitch-system-defaults.toml" cfg.defaults;
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
              cfg = config.programs.ccswitch;
            in
            {
              options.programs.ccswitch = {
                enable = lib.mkEnableOption "CCSwitch Claude Code and Codex configuration manager";
                envVars = lib.mkOption {
                  type = lib.types.nullOr lib.types.str;
                  default = null;
                  example = "%h/.config/ccswitch/env";
                  description = "Path to an environment file kept outside the Nix store for the proxy service.";
                };
                defaults = lib.mkOption {
                  type = mkDefaultsType lib pkgs;
                  default = { };
                  description = "Default Claude and Codex provider configurations";
                };
              };
              config = lib.mkIf cfg.enable {
                home.packages = [ self.packages.${pkgs.stdenv.hostPlatform.system}.default ];

                xdg.configFile."ccswitch/defaults.toml" =
                  let
                    format = pkgs.formats.toml { };
                  in
                  {
                    source = format.generate "ccswitch-defaults.toml" cfg.defaults;
                  };

                # Let CLI/TUI processes use the same out-of-store env file as
                # the proxy service without copying secrets into the Nix store.
                xdg.configFile."ccswitch/env-path" = lib.mkIf (cfg.envVars != null) {
                  text = cfg.envVars;
                };

                # Proxy service
                systemd.user.services.ccs-proxy = {
                  Unit = {
                    Description = "CCSwitch Proxy Server";
                    After = [ "network.target" ];
                  };
                  Install = {
                    WantedBy = [ "default.target" ];
                  };
                  Service = {
                    ExecStart = "${self.packages.${pkgs.stdenv.hostPlatform.system}.default}/bin/ccs proxy serve";
                    Restart = "on-failure";
                    RestartSec = "5";
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
