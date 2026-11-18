# Dockon

Dockon, [Ideavent](https://ideavent.tr/) tarafından geliştirilen ve ücretsiz sunulan bir yerel geliştirme ortamı yöneticisidir. Projelerini tek bir masaüstü uygulamasından başlatıp durdurabilir, her proje için çalışma zamanı seçebilir, yerel adresleri ve günlükleri yönetebilirsin.

Dockon aktif olarak geliştiriliyor ve topluluk katkılarına açık. Hangi özelliklerin hazır olduğunu ve platformlara göre kapsamını [geliştirme durumu](docs/progress.md) ve [uyumluluk tablosundan](docs/compatibility-matrix.md) inceleyebilirsin.

## Katkıda bulun

- **Hata bildir.** GitHub’da bir [issue](https://github.com/Ideavent/dockon/issues) aç. Dockon sürümünü, işletim sistemini, hatayı tekrar oluşturma adımlarını ve beklediğin sonucu ekle.
- **Özellik öner.** Çözmek istediğin sorunu ve özelliğin hangi iş akışında işe yarayacağını bir issue’da anlat.
- **Kod veya dokümantasyon katkısı gönder.** Değişikliğini ayrı bir dalda yap, ilgili kontrolleri çalıştır ve bir [pull request](https://github.com/Ideavent/dockon/pulls) aç. Açıklamada neyi değiştirdiğini ve nasıl doğruladığını belirt.

## Windows development

From C:\dockon_build in PowerShell:

```powershell
. .\scripts\Enter-Dev.ps1
pnpm install --frozen-lockfile
cargo build --workspace
pnpm desktop
```

Rust toolchain is installed locally in .tools on this development machine. On other machines install stable Rust, Node 24+, pnpm 11.25.0 and official Tauri 2 prerequisites. The application does not change global PATH.

Create/import a sample after building:

```powershell
.\scripts\New-SampleManifest.ps1
```

Paste the returned absolute path into Import project. Projects start only when requested. Imports do not execute code. UI and daemon run as ordinary user. Closing the UI leaves the daemon and running projects intact. Run `target\debug\dockonctl shutdown` to stop the daemon and its workloads.

## Checks

```powershell
pnpm build
cargo fmt --all -- --check
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
node scripts/smoke.mjs
node scripts/hybrid-smoke.mjs
node scripts/ui-smoke.mjs
node scripts/desktop-smoke.mjs
```

The smoke test uses an isolated temporary data directory. It verifies real IPC/auth, persistence, two-project isolation, HTTP readiness, idempotence, failure and cleanup.

The browser/native UI checks expect the Vite dev server to be running (`pnpm dev`). `desktop-smoke.mjs` is a Windows WebView2 test; its temporary debugging port is loopback-only and enabled exclusively on the isolated test process, never in normal app configuration.

`hybrid-smoke.mjs` requires a running Docker Linux engine. It downloads only digest-pinned sample images as needed, creates uniquely labeled disposable test resources, verifies two-project Redis data isolation and recovery, then removes only those verified test resources. It does not touch existing Docker projects or images. Generic container launches are implemented; managed database catalogs and backup tooling are later sprints.

Windows installed application target: C:\dockon. No production installer is available in Sprint 01. macOS/Linux builds require their own host dependencies and will be verified through CI when a GitHub remote is supplied. User projects and backups must survive uninstallation.

## Lisans

Dockon kaynak kodu [MIT Lisansı](LICENSE) kapsamında sunulur. Üçüncü taraf bileşenler kendi lisanslarına tabidir.
