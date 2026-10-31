# Typix Watch

CMCC W1A 手錶的搭配 App：配對、狀態、Wi-Fi 與 BLE 傳檔、續傳、拉 log。與手錶上的 `w1a-otad` 和 Mac 工具 `w1a-ota` 使用同一套協定。

| 資料夾 | 內容 |
|---|---|
| `core/` | Rust：協定、Noise、CPace、OTA 狀態機、分塊、續傳。無 BLE、無 UI，用 UniFFI 綁定 |
| `ios/` | Swift + UIKit，純程式碼建 UI，沒有 Storyboard |
| `android/` | Kotlin + Jetpack Compose |
| `docs/ARCHITECTURE.md` | 跨語言 callstack、執行緒、編譯連結、除錯 |

## 測試

需要 Rust。互通測試會編譯旁邊 repo 的 `linux-build/tools/w1a-otad`，路徑可用 `W1A_OTAD_DIR` 指定，找不到就略過，設 `TYPIX_REQUIRE_C=1` 改成失敗。

```sh
cd core
TYPIX_REQUIRE_C=1 cargo test

# 真的 otad.c，跑在 armv7 容器裡，需要 docker
sh tests/e2e/otad-docker.sh start
sh tests/e2e/otad-docker.sh test
sh tests/e2e/otad-docker.sh stop
```

`e2e test` 每次都要先 `start`，配對和序號不會重置。

## iOS

需要 Xcode，以及 rustup target `aarch64-apple-ios`、`aarch64-apple-ios-sim`、`x86_64-apple-ios`。

```sh
sh scripts/build-ios.sh      # core/ 改了就要重跑
open ios/TypixWatch.xcodeproj
```

### 裝到自己的 iPhone

專案沒有設定 Development Team，要在 Xcode 選你的個人帳號：

1. 先跑 `sh scripts/build-ios.sh`。
2. 開 `ios/TypixWatch.xcodeproj`，選 target **TypixWatch**，到 **Signing & Capabilities**。
3. 勾 **Automatically manage signing**，**Team** 選個人帳號。
4. **Bundle Identifier** 預設 `com.typixnode.TypixWatch`，個人帳號若撞名，改成自己的。
5. 接上 iPhone，選它當目標，執行。
6. 第一次要到 iPhone 的「設定 > 一般 > VPN 與裝置管理」信任開發者，並開啟開發者模式。

免費個人帳號的簽名 7 天過期，重新執行即可。

**不要選公司的 Team 或憑證。** Team 會寫進 `project.pbxproj`，commit 前請確認。

模擬器沒有藍牙，掃描區會顯示提示，請改填 IP。藍牙要用真機。

## Android

需要 Android SDK（platform 36）、NDK `27.1.12297006`、`cargo-ndk`，以及 rustup target `aarch64-linux-android`、`armv7-linux-androideabi`、`x86_64-linux-android`。JDK 用 Android Studio 內建的即可。

```sh
cd android
echo "sdk.dir=$HOME/Library/Android/sdk" > local.properties
export JAVA_HOME="/Applications/Android Studio.app/Contents/jbr/Contents/Home"
./gradlew assembleDebug
```

Gradle 會自己跑 `cargoNdk` 和 `uniffiBindings`，core 有變才重建。APK 在 `app/build/outputs/apk/debug/`。

Android 模擬器沒有藍牙，一樣改填 IP；host 上的服務從模擬器看是 `10.0.2.2`。

## 金鑰

每支手機有兩把長期私鑰：`identity.key`（X25519）、`signing.seed`（Ed25519）。它們等於手錶的 root：

- iOS 存 Keychain，`AfterFirstUnlockThisDeviceOnly`，不進備份和 iCloud。
- Android 存 Keystore 包裝過的值，App 關閉備份。
- 換手機就重新配對，不要搬金鑰。

repo 裡沒有任何金鑰，測試金鑰也不存檔：`cargo test` 用的輸入在執行時產生或由標籤推導，向量檔只有公開輸出。

## 使用

1. 手錶開「遠端開發」，選配對，按大鍵。
2. App 掃描到手錶，或填 IP。
3. 60 秒內輸入手錶上的六位數。輸錯一次配對就作廢。

之後每次都要先在手錶開啟並按大鍵。手錶閒置 10 分鐘、開滿 30 分鐘會自動關閉。狀態頁開著時每 5 秒查詢一次，這會維持手錶的開放時段，所以只在畫面可見時查詢。

## 授權

MIT。`core/vendor/monocypher/` 是 Monocypher 4.0.2，授權見該目錄的 `LICENCE.md`。品牌字型不在 repo 內：`scripts/build-ios.sh` 會在有 `~/w1a-build/brand` 的機器上帶入 Manrope，否則用系統字型。Nohemi 未使用。
