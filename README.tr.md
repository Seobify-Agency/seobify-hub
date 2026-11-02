# Seobify Hub

[English](README.md)

[Seobify](https://seobify.com/)'ın geliştirdiği ücretsiz ve açık kaynak SEO masaüstü araçları. Hepsi sizin bilgisayarınızda çalışır: hesap açmazsınız, kullanım takibi yoktur, arada Seobify sunucusu bulunmaz. Her araç Claude içinde MCP sunucusu olarak da çalışır; bot loglarının analizini, yönlendirme planını ya da dosya temizliğini kendi verinizle Claude'a yaptırabilirsiniz.

| | Araç | Ne yapar |
| --- | --- | --- |
| <img src="brand/seobify-log.jpg" width="40"> | **Seobify Log** | Sunucu loglarını ya da Cloudflare verisini okur; Googlebot, Bingbot ve yapay zeka botlarının sitenizi gerçekte nasıl taradığını gösterir: doğrulanmış ve sahte botlar, boşa harcanan tarama, durum kodu sorunları. |
| <img src="brand/seobify-migrate.jpg" width="40"> | **Seobify Migrate** | Site taşımada eski adresleri yenileriyle eşler, 9 sunucu ve platform için yönlendirme kuralı yazar, yayından önce ve sonra kuralları doğrular. |
| <img src="brand/seobify-optic.png" width="40"> | **Seobify Optic** | Görselleri WebP'ye çevirir; görsellerden ve DOCX, XLSX, PPTX, ODF, PDF dosyalarından gizli bilgileri (EXIF/GPS, XMP, C2PA, yapay zeka üretici verisi) temizler. |

## Kurulum

**Windows 10/11:** ihtiyacınız olan aracın kurulum dosyasını [son sürümden](https://github.com/Seobify-Agency/seobify-hub/releases/latest) indirip çalıştırın. Yalnız sizin kullanıcınıza kurulur, yönetici izni gerekmez.

**macOS:** henüz yok.

**Claude Code:** araçları kurduktan sonra eklentiyi ekleyin. Eklenti kurulu her aracı MCP sunucusu olarak tanıtır ve site taşıma iş akışını ekler.

```text
/plugin marketplace add Seobify-Agency/seobify-hub
/plugin install seobify-hub@seobify
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

Kontroller: `npm test`, `npm run build`, `cargo test --manifest-path src-tauri/Cargo.toml`. Kurulum paketi: `npm run release`. Bazı özellikler tarama aracımız Seobify Spider'a bağlanır; Spider herkese açık değildir, araçlar onsuz da çalışır.

## Geri bildirim ve güvenlik

Hata bildirimlerinizi ve önerilerinizi [Issues](https://github.com/Seobify-Agency/seobify-hub/issues) bölümüne yazabilirsiniz; bu topluluk desteğidir, yanıt süresi taahhüdü yoktur. Şimdilik kod katkısı (pull request) kabul etmiyoruz, ayrıntı [CONTRIBUTING.md](CONTRIBUTING.md) dosyasında. Güvenlik sorunlarını [SECURITY.md](SECURITY.md) dosyasındaki adrese gizli olarak bildirin.

## Lisans

Kod [GNU Affero Genel Kamu Lisansı v3.0](LICENSE) ile lisanslanmıştır. Seobify adı ve logoları lisansın kapsamında değildir, bkz. [TRADEMARKS.md](TRADEMARKS.md).

Site taşıması planlıyorsanız ya da araçların bulduklarını bir uzmanın uygulamasını istiyorsanız [Seobify ile görüşün](https://seobify.com/iletisim/?utm_source=github&utm_medium=readme&utm_campaign=seobify-hub).
