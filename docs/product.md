# Product

## Vision

**Your phone and every other device, connected. No cables, no cloud, no limits.**

Moving things between a phone and a computer (or another phone) is still clumsy: people email
files to themselves, send photos to their own WhatsApp (which compresses them), hunt for cables,
or upload gigabytes to the cloud just to download them a metre away. Apple users have AirDrop;
everyone else has a patchwork of tools that each solve part of the problem.

Beam is a **connectivity tool**. File transfer is the entry point; continuity between devices
(clipboard, and later folders, notifications, camera) is what makes people stay.

## Who it is for

Everyone with a smartphone. There is no narrow target segment: anyone who ever needs to move
something from their phone to a PC, another phone, a tablet or a TV is a user.

## Positioning against existing tools

| Tool | Weakness | Beam's answer |
|---|---|---|
| Quick Share | Tied to Google/Samsung, flaky across brands, no browser, no continuity | Works with any device and any browser |
| LocalSend | Needs the app on both sides, plain UI, transfer only | Zero install on the receiver, polished UX, continuity |
| SHAREit / Xender | Ads, bloat, trust problems | Clean, private, fast |
| Phone Link | Windows only, inconsistent | Every platform, reliable |
| KDE Connect | Powerful but technical | The same power, made simple |

## Principles

1. **Fast:** limited by the link, not by the app. Speeds are published as benchmarks.
2. **Private:** data goes device to device, end-to-end encrypted. No account required.
3. **Reliable:** transfers resume after drops and survive the app going to the background.
4. **Effortless:** the receiving side needs nothing installed; pairing is one QR scan.
5. **Beautiful:** a polished, animated interface that feels native and premium.

## Platforms

- **Android:** native app (the primary product).
- **Everything else:** a Progressive Web App (PWA) in any modern browser, installable on desktop.
- **Later:** native desktop apps (Windows, macOS, Linux) and possibly iOS.

## Scope strategy

Start **focused** (best-in-class transfer + clipboard), then grow toward a **full continuity
platform**. There is no fixed deadline; quality and reliability come first.

## v1 scope

### Transfer
- Phone ↔ browser (PWA), with nothing installed on the receiving side.
- Phone ↔ phone (Android ↔ Android).
- **Direct Wi-Fi mode:** Wi-Fi Direct / Wi-Fi Aware, bypassing the router.
- **USB-C cable mode:** a USB network link for multi-gigabit transfers.
- **Offline mode:** a local web server when there is no internet.
- Original quality, no size limits, files and folders.
- Resumable transfers with per-chunk integrity checks.
- Share sheet integration ("Share → Beam") and transfer history.

### Continuity
- **Universal clipboard** between phone and browser (text, images, files).

### Trust
- QR pairing with key verification; trusted devices remembered.
- End-to-end encryption by default.
- No account.

## Later versions

| Version | Features |
|---|---|
| v1.1 | LocalSend protocol compatibility |
| v2 | Native desktop apps, live folder sync, automatic photo backup to PC, notification mirroring |
| v3 | Phone as webcam and microphone, screen mirroring and control, broadcast to many receivers, internet relay across networks, automations, CLI |

## Monetization

- **Free:** unlimited transfers, browser sharing, phone ↔ phone, direct Wi-Fi and cable modes.
- **Pro (subscription + lifetime option):** universal clipboard at launch; folder sync, photo
  backup, desktop continuity, webcam and other continuity features as they ship; unlimited relay.
- **Ads:** none during transfers. At most a small banner in transfer history on the free tier.

## Decisions log

| Decision | Choice |
|---|---|
| Accounts | None in v1; pairing by QR only |
| Relay (TURN) | Free with a monthly data cap, unlimited on Pro |
| LocalSend compatibility | v1.1 |
| Web app framework | React (Vite, static PWA); shares a design system with the website |
| Marketing website | Next.js with GSAP and anime.js for animations |
| Name | "Beam" is a codename; public name chosen before launch |
