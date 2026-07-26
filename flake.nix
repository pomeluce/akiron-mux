{
  description = "CCSwitch — Claude Code model configuration manager";

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
            version = "1.9.0";
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

      flake = {
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
              enable = lib.mkEnableOption "CCSwitch model configuration manager";
              defaults = lib.mkOption {
                type = lib.types.attrs;
                default = { };
                description = "Provider configurations (written to /etc/ccswitch/defaults.toml)";
              };
            };
            config = lib.mkIf cfg.enable {
              environment.systemPackages = [ self.packages.${pkgs.system}.default ];
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
              enable = lib.mkEnableOption "CCSwitch model configuration manager";
              envVars = lib.mkOption {
                type = lib.types.nullOr (lib.types.either lib.types.str (lib.types.attrsOf lib.types.str));
                default = null;
                example = {
                  DEEPSEEK_API_KEY = "sk-xxx";
                };
                description = "Environment file path or attrset of env vars for proxy service.";
              };
              defaults = lib.mkOption {
                type =
                  let
                    format = pkgs.formats.toml { };
                  in
                  lib.types.submodule {
                    freeformType = format.type;
                    options.version = lib.mkOption {
                      type = lib.types.int;
                      default = 1;
                    };
                    options.providers = lib.mkOption {
                      type = lib.types.listOf (
                        lib.types.submodule {
                          freeformType = format.type;
                          options = {
                            id = lib.mkOption { type = lib.types.str; };
                            name = lib.mkOption { type = lib.types.str; };
                            api_url = lib.mkOption { type = lib.types.str; };
                            api_key = lib.mkOption { type = lib.types.str; };
                            profiles = lib.mkOption {
                              type = lib.types.listOf (
                                lib.types.submodule {
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
                                }
                              );
                              default = [ ];
                            };
                          };
                        }
                      );
                      default = [ ];
                    };
                  };
                default = { };
                description = "Default provider configurations";
              };
            };
            config = lib.mkIf cfg.enable {
              home.packages = [ self.packages.${pkgs.system}.default ];

              xdg.configFile."ccswitch/defaults.toml" =
                let
                  format = pkgs.formats.toml { };
                in
                {
                  source = format.generate "ccswitch-defaults.toml" cfg.defaults;
                };

              # Env file: attrset → generate file; string → use as-is
              xdg.configFile."ccswitch/env" = lib.mkIf (cfg.envVars != null && !lib.isString cfg.envVars) {
                text = lib.concatStrings (lib.mapAttrsToList (k: v: "${k}=${v}\n") cfg.envVars);
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
                  ExecStart = "${self.packages.${pkgs.system}.default}/bin/ccs proxy serve";
                  Restart = "on-failure";
                  RestartSec = "5";
                }
                // lib.optionalAttrs (cfg.envVars != null) {
                  EnvironmentFile = if lib.isString cfg.envVars then cfg.envVars else "%h/.config/ccswitch/env";
                };
              };
            };
          };
      };
    };
}
