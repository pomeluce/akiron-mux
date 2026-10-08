# Automate tag releases and synchronize verified Nix metadata

Release 保持“推送 Tag → 校验版本并构建 → 直接发布”的单次运行流程，也可填写目标 Tag 手动补发。各构建任务固定使用校验后的 Tag 提交，发布任务等待全部构建成功、直接汇总产物并生成真实 SHA-256 的 Nix 清单；禁止覆盖已有资产。

独立候选阶段、候选 run ID 和 SSH 补发触发分支的维护成本高于本项目的收益，因此移除。正常 Tag 推送使用 SSH Git 认证即可；旧 Tag 补发保留网页手动入口，不为此维护额外的长期发布协议。

产物 hash 只能在构建完成后获得，因此不要求源码 Tag 预先包含新版 Nix 清单。清单随 Release 发布；Nix 同步任务重新下载真实发布文件并验证 hash，通过独立 PR 同步默认分支，不移动 Tag、不覆盖同版本清单、不降级到较旧版本。Nix 同步和 Homebrew 更新独立于主发布任务，失败不影响已发布资产。代价是固定源码 Tag 中的 Nix 清单可能仍指向上一版，使用新版二进制的 Nix 用户需跟踪清单同步后的提交。
