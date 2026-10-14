# ondaluxheatmap.com

Static website for Ondalux Heatmap: English at `/`, Spanish at `/es/`.
No frameworks and no third-party requests (fonts, analytics or CDNs).

```
website/
  src/template.html   page template (both languages)
  src/en.json, es.json texts
  build.mjs           generates public/index.html, public/es/index.html,
                      sitemap.xml and copies ../dist/ondalux.exe to
                      public/downloads/ondalux-heatmap-<version>-windows-x64.exe
                      (size and SHA-256 go into the pages)
  public/             what is uploaded to public_html (assets, .htaccess, 404)
  serve.mjs           local preview: node website/serve.mjs  → http://localhost:4173
  deploy.ps1          build + upload over FTPS, or -Zip for cPanel
```

## Publish a new version

1. Build the app: `cargo build --release -p ondalux-app` and copy
   `target/release/ondalux.exe` to `dist/`.
2. Then either:
   - **Zip for cPanel**: `.\website\deploy.ps1 -Zip`, then in cPanel
     *File Manager → public_html → Upload* the zip and *Extract* it.
   - **FTP upload**: set `$env:ONDALUX_FTP_HOST` and `$env:ONDALUX_FTP_USER`
     (cPanel → FTP Accounts) and run `.\website\deploy.ps1`; the password is
     asked for at the prompt and never saved. Use `-RemoteDir ""` if the FTP
     account already opens in `public_html`.

Screenshots come from the app's UI test
(`ONDALUX_LANG=en|es ONDALUX_SHOTS=… cargo test -p ondalux-app ui_shots -- --ignored`)
and the sample reports from `cargo run -p ondalux-report --example report`.

After the first publish, enable SSL for the domain in cPanel (*SSL/TLS
Status → Run AutoSSL*); `.htaccess` redirects every request to
`https://ondaluxheatmap.com`.

## Configuration (`src/site.json`)

| Key | Use |
|---|---|
| `contact` | Support e-mail shown in the footer, the privacy policy and the open-source page. |
| `github` | Repository URL (e.g. `https://github.com/user/ondalux-heatmap`). Empty = "coming soon"; when set, the open-source page links the repo, issues and docs, and the footer shows GitHub. |
| `ads.client` | Google AdSense publisher ID (`ca-pub-…`). Empty = no ads, no Google code, no cookies. When set, every page gets the AdSense script and `google-adsense-account` meta, `ads.txt` is generated, the privacy policy gains its advertising section and the footer gets "Cookie settings". |
| `ads.slots.*` | Ad unit IDs for the reserved spaces (`home_top`, `home_middle`, `home_bottom`, `guide_top`, `guide_bottom`, `oss_middle`). Empty slots render nothing; with no slot IDs at all, use *Auto ads* in AdSense instead. |
| `ads.preview` | `true` (or `ADS_PREVIEW=1 node website/build.mjs`) draws dashed placeholders where the ads go, to review the layout. |

### Getting approved by AdSense

1. Publish the site (with HTTPS working) at https://ondaluxheatmap.com.
2. Create the AdSense account, add the site and copy the publisher ID into
   `ads.client`; rebuild and publish (this adds the verification code and
   `ads.txt`).
3. In AdSense → *Privacy & messaging*, create the **European regulations**
   message (Google's certified consent banner, required for visitors from
   the EEA, UK and Switzerland). No code change is needed; the footer link
   "Cookie settings" reopens it.
4. Request the review. After approval, create display ad units and paste
   their IDs into `ads.slots`, or enable Auto ads.
