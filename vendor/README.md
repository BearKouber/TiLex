# vendor/

## wcocr.dll (167 KB)

微信 OCR 的驱动层，从 https://github.com/swigger/wechat-ocr 编出来的。
它自己不含任何识别能力，只负责用 mmmojo IPC 驱动**用户本机已装的微信**：

- 引擎 `%APPDATA%\Tencent\xwechat\XPlugin\Plugins\WeChatOcr\<版本>\extracted\wxocr.dll`
- 运行时 `<微信安装目录>\<版本>\`（注册表 `HKCU\Software\Tencent\Weixin` 的 `InstallPath`）

这两样都留在原地，我们不复制、不打包。没装微信就是没有 OCR。

导出两个函数，用法见 `src-tauri/src/ocr.rs`：

```c
bool wechat_ocr(LPCTSTR ocr_exe, LPCTSTR wechat_dir,
                const char* imgfn, void(*set_res)(const char* json));
void stop_ocr();
```

## 怎么重新编（本机工具链就够，不用装东西）

```bash
git clone --depth 1 https://github.com/swigger/wechat-ocr.git
cd wechat-ocr
# 删掉 CMakeLists.txt 里 pywcocr 那个目标：它 find_package(Python ... Development REQUIRED)，
# 本机没 Python 开发头文件，不删配置阶段就失败。只留 wcocr 和 test_cli。
CMAKE="/c/Program Files (x86)/Microsoft Visual Studio/2022/BuildTools/Common7/IDE/CommonExtensions/Microsoft/CMake/CMake/bin/cmake.exe"
"$CMAKE" -S . -B build -G "Visual Studio 17 2022" -A x64
"$CMAKE" --build build --config Release
# 产物：build/Release/wcocr.dll
```

protobuf 是仓库里 vendored 的（`spt/x64`），不用下。
