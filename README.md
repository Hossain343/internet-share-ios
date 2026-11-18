# WiFi Share (iOS, Rust core)

اینترنت Wi-Fi گوشی را از طریق کابل USB به کامپیوتر می‌دهد.

## نحوه کار
iOS به اپ‌ها اجازه نمی‌دهد خودشان را به‌عنوان کارت شبکه USB به PC معرفی کنند.
راه سریع و مجاز: اپ یک پروکسی SOCKS5 روی گوشی (127.0.0.1:1080) اجرا می‌کند و
ترافیک خروجی را فقط از Wi-Fi (en0) می‌فرستد. کامپیوتر از طریق usbmuxd (همان
مکانیزم iTunes) پورت را روی کابل فوروارد می‌کند.

## بیلد
1. کل این پوشه را در یک ریپوی GitHub پوش کن.
2. تب Actions → Build iOS IPA → Artifacts → `WiFiShare-ipa`.
3. IPA بدون امضاست؛ با Sideloadly / AltStore / TrollStore نصب کن.

## استفاده
1. اپ را روی گوشی باز کن (خودکار شروع می‌شود). لوکیشن را Allow کن تا اسم Wi-Fi نمایش داده شود.
2. گوشی را با کابل وصل کن.
3. روی PC (Windows: iTunes یا Apple Devices نصب باشد + `iproxy` از libimobiledevice):
   `iproxy 1080 1080`
4. روی PC پروکسی SOCKS5 را `127.0.0.1:1080` بگذار. برای کل سیستم: Proxifier یا tun2socks+Wintun.
   (روی Mac/Linux: `iproxy 1080:1080`)

## نکات
- فقط TCP (CONNECT) پشتیبانی می‌شود؛ DNS روی گوشی resolve می‌شود (SOCKS5h / domain).
- نمایش اسم Wi-Fi به entitlement «Access WiFi Information» نیاز دارد؛ با اکانت رایگان ممکن است «نامشخص» نشان دهد.
- حالت بک‌گراند با پخش صدای بی‌صدا حفظ می‌شود.
