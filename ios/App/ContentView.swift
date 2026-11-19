import SwiftUI

struct ContentView: View {
    @ObservedObject var model: Model

    var body: some View {
        VStack(alignment: .leading, spacing: 18) {
            row("Wi-Fi name:", model.ssid)
            row("وضعیت:", model.connected ? "وصل" : "قطع")
            row("ipaddrs:", model.ip)
            Divider()
            row("اسم سیستم:", model.pcName)
            row("اتصال به برنامه سیستم:", model.linked ? "وصل" : "قطع")
            Divider()
            row("Proxy (SOCKS5):", model.running ? "127.0.0.1:\(model.port)" : "خاموش")
            row("اتصالات فعال:", "\(model.active)")
            Spacer()
            Button(action: { model.toggle() }) {
                Text(model.running ? "توقف" : "شروع")
                    .frame(maxWidth: .infinity)
                    .padding()
                    .background(model.running ? Color.red : Color.green)
                    .foregroundColor(.white)
                    .cornerRadius(12)
            }
        }
        .padding(24)
    }

    private func row(_ k: String, _ v: String) -> some View {
        HStack {
            Text(k).font(.headline)
            Spacer()
            Text(v).font(.system(.body, design: .monospaced))
        }
    }
}
