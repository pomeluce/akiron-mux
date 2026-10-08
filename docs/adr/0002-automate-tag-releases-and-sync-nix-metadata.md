# Automate tag releases and synchronize verified Nix metadata

为恢复推送 Tag 即发布的行为，Release 使用目标 Tag 的固定源码提交构建，在同一次运行中校验并发布保存的原始产物；手动 release 可补发已有 Tag，prepare/publish 可分开运行。候选产物记录源码提交，禁止发布到其他 Tag，也禁止覆盖已有资产。

产物 hash 只能在构建完成后获得，因此不再要求源码 Tag 预先包含新版 Nix 清单。清单随 Release 发布，并通过独立 PR 同步默认分支；同步只修改清单，不移动 Tag，也不将默认分支的清单降级到较旧版本。代价是固定源码 Tag 中的 Nix 清单可能仍指向上一版，使用新版二进制的 Nix 用户需跟踪清单同步后的提交。
