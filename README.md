# Font Loader

一个 Windows 临时字体加载器。将字体文件拖入窗口后，它会在 `Font Loader` 保持运行期间加载字体；关闭窗口会自动释放全部仍在列表中的字体。

## 下载

| Platform | Architecture | Package | Hash |
| --- | --- | --- | --- |
| Windows | x86_64 | [Download](https://github.com/Joey-Kot/FontLoader/releases/download/Latest/FontLoader-windows-x86_64.zip) | [sha256](https://github.com/Joey-Kot/FontLoader/releases/download/Latest/FontLoader-windows-x86_64.zip.sha256) |
| Windows | arm64 | [Download](https://github.com/Joey-Kot/FontLoader/releases/download/Latest/FontLoader-windows-arm64.zip) | [sha256](https://github.com/Joey-Kot/FontLoader/releases/download/Latest/FontLoader-windows-arm64.zip.sha256) |

## 使用

1. 打开 `FontLoader.exe`。
2. 将字体文件拖进窗口，或将文件夹拖入窗口。文件夹会递归扫描 `.ttf`、`.otf`、`.ttc`、`.fon`、`.fnt` 和 `.fot` 文件。
3. 单击一行选中字体；按住 `Ctrl` 或 `Shift`，也可以框选多行。
4. 按 `Delete` 或 `Backspace` 卸载选中的字体。
5. 关闭窗口会卸载其余已加载字体。
