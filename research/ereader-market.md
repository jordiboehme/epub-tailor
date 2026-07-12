# Which e-readers are worth a profile

Researched 2026-07-12. This file exists to record *why* we ship the profiles we ship, and - just as importantly - what we refused to claim.

## There is no credible market-share data for e-ink readers

Searching for it produces confident-looking numbers. They do not survive contact:

- "Kindle has 65-70% market share" - traces to affiliate blogs with no stated methodology.
- "Amazon, Kobo and Sony collectively hold about 88%" - a report listing **Sony**, which exited the e-reader business years ago, as a top-three player in 2026. Recycled boilerplate.
- Mordor / SkyQuest / MarketResearchFuture - paywalled lead-generation reports; the free snippets carry no brand percentages at all.

The only reasonably-sourced share figure found anywhere is German, and it measures the wrong thing: GfK-derived data showing tolino at roughly 45% against Kindle's 39% of the German **e-book sales** market (not device share), and several years stale.

**So we cite no percentages.** If a number appears nowhere credible, the honest move is to say so rather than launder a guess into a doc.

## What we used instead: editorial consensus

Every 2026 "best e-reader" roundup - TechRadar, Engadget, Good e-Reader, Forbes - converges on the same short list. That is a weaker signal than sales data, but it is a real one, and it is labelled as what it is.

| Priority | Brand | Why | Status |
|---|---|---|---|
| 1 | **Amazon Kindle** | The default in every roundup. Paperwhite 12th gen is the single most-cited "best for most people". | shipped |
| 2 | **Rakuten Kobo** | The recommended alternative, and *Engadget's best-overall pick is a Kobo*. Best EPUB citizen: publishes a real publisher spec, sideload-friendly. | shipped |
| 3 | **Onyx Boox** | The enthusiast pick (Palma 2 Pro, Go Color 7, Page). Android, so people run whatever app they like. | shipped |
| 4 | **PocketBook** | Europe-weighted, and its users are *locked into* the stock reader by Adobe DRM - so tailoring lands. | shipped |
| 5 | **tolino** | Regionally dominant in Germany/DACH, invisible elsewhere. | shipped |
| 6 | **Barnes & Noble Nook** | **Skipped.** A zombie: nothing new shipped, retired devices being cut off, absent from every roundup. Two e-ink Nooks were *announced* for 2026; none has shipped. | skipped |
| 7 | **reMarkable** | **Skipped.** A note-taker, not an e-reader. DRM-free EPUB only, a basic reading app, and it does not even appear in Engadget's e-reader guide (it lives in the separate "E Ink tablet" category). | skipped |

Also skipped: Bigme, Viwoods, Meebook, Hanvon - long-tail, no evidence of relevance.

## The 2026 trend that mattered for the code

**Colour went mainstream.** Kaleido 3 is now on the Kindle Colorsoft, the Kobo Clara/Libra Colour, the tolino colour line, the PocketBook colour line and half the Boox range. That is what forced the image pipeline to stop assuming grayscale, and it is why `panel` (`gray4` / `gray16` / `color`) is a first-class device capability rather than an afterthought.

Every Kaleido 3 device shares the same shape: the panel's mono resolution at 300 PPI, with colour rendered at **half the linear density (150 PPI)**. We fit images to the full mono resolution and keep them RGB; the device does its own colour subsampling.

## Sources

TechRadar "best ereader" · Engadget "best ereaders for 2026" (2026-01-29) and "best E Ink tablets" · Good e-Reader "Top 8 for Spring 2026" · Forbes "best e-readers" · Barnes & Noble press releases and the retired-NOOK-devices help page · reMarkable's own Ebooks support doc (DRM-free EPUB only) · Statista / lesen.net (the German e-book share figure, and its limits).
