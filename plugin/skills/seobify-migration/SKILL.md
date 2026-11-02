---
name: seobify-migration
description: Site migration runbook with Seobify Hub tools — map old URLs to new ones with Seobify Migrate, export redirect rules, verify them before and after launch, and watch rankings and crawling afterwards with Watch, Console and Log. Use when the user plans or runs a site migration, domain change, CMS move, URL restructure, "site taşıma", "301 yönlendirme planı", "redirect haritası", or checks a migration after launch.
---

> Herkese açık sürümde Seobify Log, Migrate ve Optic var. `seobify-watch` ya da `seobify-console` araçları yoksa o adımları atla ve raporda bunu belirt.

# Seobify site taşıma akışı

Taşıma üç evrede ilerler. Her evrede kullanıcıya ne yapıldığını, sayıları ve bir sonraki kararı söyle.

## 1. Yayın öncesi

1. Spider'da eski sitenin ve (varsa) staging'in taramalarını belirle (`spider_projects`).
2. `seobify-migrate`: `create_plan(project, name, old_base, new_base, staging_base)`.
3. `import_urls(plan, side="old", source={kind:"spider", ...})` ve yeni taraf için aynısı (staging taraması, sitemap ya da CSV).
4. Değerli URL'leri öne almak için Console verisi varsa: `seobify-console` `performance(project, dimension="page", from=son 90 gün)` → yüksek tıklamalı sayfaları kullanıcıya göster.
5. `auto_match(plan)` → durum dağılımını raporla. `list_mappings(plan, status="review")` ve `status="weak"` ile kullanıcıyla gözden geçir; desen önerilerini `add_pattern` ile uygula, kararları `set_mapping` ile kaydet.
6. `validate_plan(plan)` → zincir, döngü, eksik hedef sıfır olana kadar düzelt.
7. `verify(plan, kind="staging")` → staging hedeflerinin 200 verdiğini doğrula.
8. `export_rules(plan, format=<sunucu>)` → dosyayı kullanıcıya ver; sunucu tipini sor (apache, nginx, iis, cloudflare, netlify, vercel, nextjs, wordpress).

## 2. Yayın günü

1. `verify(plan, kind="post")` → başarı oranı ve hatalar; hataları kategoriye göre (yanlış hedef, zincir, 302, yönlendirilmemiş, hedef hatası) listele.
2. `seobify-watch`: en değerli 50–200 yeni URL için `create_group` (15 dk) + `add_monitors`; eski alan adının `robots.txt`'si için izleyici.
3. `seobify-console`: taşıma tarihini not et; `sync_now`.

## 3. Yayın sonrası (ilk 4 hafta, haftalık)

1. `seobify-migrate` `verify` tekrarı ve önceki çalıştırmayla karşılaştırma.
2. `seobify-log`: Googlebot'un eski URL'lere istekleri (`top_urls` bot=`googlebot`, status=`3xx` ve `4xx`), yeni URL'lerin taranma hızı (`spider_gaps` yeni taramayla).
3. `seobify-console`: `compare_periods` (taşıma öncesi 28 gün vs sonrası), `anomalies`, kaybeden sayfalar → eşleme ya da içerik sorunu ayrımı.
4. Rapor: taşıma sağlığı (doğrulama oranı, Googlebot geçişi, trafik farkı), açık sorunlar, eylemler.
