# STS2 AI Teammate

> AI 控制的队友，陪你单刷杀戮尖塔 2。
> Forked from [SallyHong2347/sts2-ai-teammate](https://github.com/SallyHong2347/sts2-ai-teammate)，适配当前游戏版本。

## 功能

- 在本地 fake-multiplayer 跑 AI 队友
- AI 自动战斗、选牌、选遗物、用药水、逛商店、处理事件
- 每角色独立行为配置
- 纯 C#，无 Godot 场景依赖

## 安装

下载 Release 中的 `STS2_AiTeammate.zip`，解压到 `Mods/` 目录。

## 构建

```bash
dotnet build STS2-AiTeammate.csproj -c Release
```

需要 .NET 9.0 SDK 和已安装的 Slay the Spire 2。

## 许可

MIT