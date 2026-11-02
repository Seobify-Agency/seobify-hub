# Seobify Hub

[English](README.md)

[Seobify](https://seobify.com/)'ın geliştirdiği ücretsiz ve açık kaynak SEO masaüstü araçları. Hepsi sizin bilgisayarınızda çalışır: hesap açmazsınız, kullanım takibi yoktur, arada Seobify sunucusu bulunmaz. Her araç Claude içinde MCP sunucusu olarak da çalışır; bot loglarının analizini, yönlendirme planını ya da dosya temizliğini kendi verinizle Claude'a yaptırabilirsiniz.

| | Araç | Ne yapar |
| --- | --- | --- |
| <img src="brand/seobify-log.jpg" width="40"> | **Seobify Log** | Sunucu loglarını ya da Cloudflare verisini okur; Googlebot, Bingbot ve yapay zeka botlarının sitenizi gerçekte nasıl taradığını gösterir: doğrulanmış ve sahte botlar, boşa harcanan tarama, durum kodu sorunları. |
| <img src="brand/seobify-migrate.jpg" width="40"> | **Seobify Migrate** | Site taşımada eski adresleri yenileriyle eşler, 9 sunucu ve platform için yönlendirme kuralı yazar, yayından önce ve sonra kuralları doğrular. |
| <img src="brand/seobify-optic.png" width="40"> | **Seobify Optic** | Görselleri WebP'ye çevirir; görsellerden ve DOCX, XLSX, PPTX, ODF, PDF dosyalarından gizli bilgileri (EXIF/GPS, XMP, C2PA, yapay zeka üretici verisi) temizler. |

## Kurulum

**Windows 10/11:** ihtiyacınız olan aracın kurulum dosyasını [sürümler sayfasından](https://github.com/Seobify-Agency/seobify-hub/releases) indirip çalıştırın. Yalnız sizin kullanıcınıza kurulur, yönetici izni gerekmez. Kurulum dosyaları henüz kod imzalı değil: Windows "Windows bilgisayarınızı korudu" uyarısı gösterirse "Ek bilgi" → "Yine de çalıştır" deyin. İmzalı sürümler ardından gelecek.

**macOS:** henüz yok.

**Claude Code:** araçları kurduktan sonra eklentiyi ekleyin. Eklenti kurulu her aracı MCP sunucusu olarak tanıtır ve site taşıma iş akışını ekler.

```text
/plugin marketplace add Seobify-Agency/seobify-hub
/plugin install seobify-hub@seobify
```

## Diğer yapay zeka araçlarıyla kullanım

Her araç standart bir MCP sunucusudur (stdio): kurulu program `--mcp` ile başlatılır. Her uygulamada Ayarlar → Claude / MCP bölümü bilgisayarınıza özel komutu ve JSON'u gösterir. Aşağıdaki yollar varsayılan kullanıcı kurulumuna göredir.

**OpenAI Codex**

```text
codex mcp add seobify-log -- "%LOCALAPPDATA%\Seobify Log\seobify-log.exe" --mcp
codex mcp add seobify-migrate -- "%LOCALAPPDATA%\Seobify Migrate\seobify-migrate.exe" --mcp
codex mcp add seobify-optic -- "%LOCALAPPDATA%\Seobify Optic\seobify-optic.exe" --mcp
```

**Gemini CLI** (`~/.gemini/settings.json`), **Google Antigravity** (MCP sunucuları → ham yapılandırma), **Claude Desktop** (`claude_desktop_config.json`), **Cursor** (`~/.cursor/mcp.json`) ve diğer MCP istemcileri aynı JSON'u kullanır. `<kullanıcı>` yerine Windows kullanıcı adınızı yazın:

```json
{
  "mcpServers": {
    "seobify-log": { "command": "C:\\Users\\<kullanıcı>\\AppData\\Local\\Seobify Log\\seobify-log.exe", "args": ["--mcp"] },
    "seobify-migrate": { "command": "C:\\Users\\<kullanıcı>\\AppData\\Local\\Seobify Migrate\\seobify-migrate.exe", "args": ["--mcp"] },
    "seobify-optic": { "command": "C:\\Users\\<kullanıcı>\\AppData\\Local\\Seobify Optic\\seobify-optic.exe", "args": ["--mcp"] }
  }
}
```

## Gizlilik

- Veriler `~/Seobify/<Araç>` klasöründe kalır (Windows'ta `C:\Users\<kullanıcı>\Seobify\<Araç>`). Aracı kaldırmak verileri silmez.
- Erişim belirteçleri (örneğin Cloudflare API anahtarı) Windows Kimlik Bilgisi Yöneticisi'nde saklanır, yalnız maskeli gösterilir.
- Araçlar yalnız sizin belirttiğiniz sitelere ve bağladığınız hizmetlere bağlanır. Analitik ya da takip kodu yoktur.
- Raporların sonunda Seobify'a yönlendiren tek bir satır bulunur; Ayarlar → Hakkında bölümünden kapatabilirsiniz.

## Kaynaktan derleme

Her araç `apps/<araç>` altında bir [Tauri 2](https://tauri.app) uygulamasıdır (Rust, React, TypeScript):

```text
cd apps/log
npm install
npm run tauri dev
```

Kontroller: `npm test`, `npm run build`, `cargo test --manifest-path src-tauri/Cargo.toml`. Kurulum paketi: `npm run release`. Bazı özellikler ücretsiz tarama aracımız Seobify Spider'ın taramalarını okur ([indir](https://github.com/Seobify-Agency/seobify-spider-releases/releases)); araçlar onsuz da çalışır.

## Geri bildirim ve güvenlik

Hata bildirimlerinizi ve önerilerinizi [Issues](https://github.com/Seobify-Agency/seobify-hub/issues) bölümüne yazabilirsiniz; bu topluluk desteğidir, yanıt süresi taahhüdü yoktur. Şimdilik kod katkısı (pull request) kabul etmiyoruz, ayrıntı [CONTRIBUTING.md](CONTRIBUTING.md) dosyasında. Güvenlik sorunlarını [SECURITY.md](SECURITY.md) dosyasındaki adrese gizli olarak bildirin.

## Lisans

Kod [GNU Affero Genel Kamu Lisansı v3.0](LICENSE) ile lisanslanmıştır. Seobify adı ve logoları lisansın kapsamında değildir, bkz. [TRADEMARKS.md](TRADEMARKS.md).

Site taşıması planlıyorsanız ya da araçların bulduklarını bir uzmanın uygulamasını istiyorsanız [Seobify ile görüşün](https://seobify.com/iletisim/?utm_source=github&utm_medium=readme&utm_campaign=seobify-hub).
