# WiFi Share — اینترنت Wi-Fi گوشی (iPhone) برای ویندوز از طریق کابل

## اجزا
- **iPhone app** (Rust core + SwiftUI): پروکسی SOCKS5 روی 127.0.0.1:1080 (خروجی فقط از Wi-Fi) + کانال کنترل 1081 برای هندشیک.
- **Windows Receiver** (Rust): با usbmuxd مستقیم به گوشی وصل می‌شود، هندشیک می‌کند، و یک کارت شبکه مجازی (Wintun, نام `WiFiShare`) می‌سازد که تمام ترافیک ویندوز را از گوشی عبور می‌دهد. DNS هم از همین مسیر عبور می‌کند.

## بیلد
پوش به GitHub → تب Actions:
- `Build iOS IPA` → artifact: `WiFiShare-ipa` (بدون امضا؛ با Sideloadly/AltStore نصب کن)
- `Build Windows Receiver` → artifact: `WiFiSharePC-windows` (فایل `WiFiSharePC.exe`)

## استفاده
1. روی PC یکی از این‌ها نصب باشد: iTunes یا Apple Devices (برای Apple Mobile Device Service).
2. گوشی را با کابل وصل کن، قفلش را باز کن، Trust را بزن. اپ گوشی را باز کن (Wi-Fi گوشی روشن باشد).
3. `WiFiSharePC.exe` را اجرا کن (درخواست Administrator می‌دهد). وضعیت می‌شود Connected و اینترنت ویندوز از گوشی می‌آید.

## محدودیت‌ها
- فقط TCP (و DNS) عبور می‌کند؛ UDP عمومی (QUIC/بازی) پشتیبانی نمی‌شود.
- نمایش اسم Wi-Fi به entitlement اپل نیاز دارد؛ با اکانت رایگان ممکن است «نامشخص» باشد.
- tun2socks و Wintun هنگام بیلد دانلود و داخل exe ادغام می‌شوند.

## عیب‌یابی
- اگر برنامه ویندوز نوشت Apple USB support not installed: دکمه Install را بزن (iTunes را با winget نصب می‌کند) یا Apple Devices را از Microsoft Store نصب کن. بعد از نصب خودش تشخیص می‌دهد.
- اگر سرویس Apple Mobile Device Service خاموش باشد برنامه خودش آن را روشن می‌کند.
- اگر اپ گوشی خطا نشان داد، متن قرمز را بفرست.
