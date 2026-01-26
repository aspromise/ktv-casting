# 与[ktv-song-web](https://github.com/StarFreedomX/ktv-song-web)搭配的命令行DLNA投屏软件

[开发者文档](docs/DEVELOPER.md)
## 使用方式

输入搭建好的[ktv-song-web](https://github.com/StarFreedomX/ktv-song-web)服务的网址（含对应房间编号），如`http://ktv.example.com/101`，随后选择搜索到的DLNA设备，即可使用。

## 功能

跟随网页的正在播放曲目进行投屏，结束自动切歌。也可以在网页端操作进行切歌。

## 手机上怎么用

1. 下载并安装[Termux](https://termux.com/)。
2. 从[这里](https://github.com/aspromise/ktv-casting/releases)下载最新的`ktv-casting-aarch64-linux-android`可执行文件。
建议可以直接在Termux中使用`curl -LO <下载链接>`命令下载。以`v0.1.5`版本为例，命令如下：
```bash
curl -LO https://github.com/aspromise/ktv-casting/releases/download/v0.1.5/ktv-casting-aarch64-linux-android
```
3. 赋予可执行权限：
```bash
chmod +x ktv-casting-aarch64-linux-android
```
4. 运行程序：
```bash
./ktv-casting-aarch64-linux-android
```

## 保存日志

默认情况下，程序不会保存日志文件。如果需要保存日志，请设置环境变量`KTV_LOG`为`true`，这将在运行目录下生成`ktv-casting.log`日志文件。

Linux中设置方法如下：
```bash
export KTV_LOG=true
```

Windows中设置方法如下：
```powershell
$env:KTV_LOG="true"
```