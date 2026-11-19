import SwiftUI
import UIKit
import NetworkExtension
import CoreLocation
import AVFoundation
import Darwin

final class Model: NSObject, ObservableObject, CLLocationManagerDelegate {
    @Published var ssid = "—"
    @Published var connected = false
    @Published var ip = "—"
    @Published var running = false
    @Published var active = 0
    @Published var linked = false
    @Published var pcName = "—"
    @Published var error = ""

    let port: UInt16 = 1080
    private let lm = CLLocationManager()
    private var timer: Timer?
    private var player: AVAudioPlayer?
    private var tick = 0

    override init() {
        super.init()
        lm.delegate = self
        lm.requestWhenInUseAuthorization()
        UIDevice.current.name.withCString { wifi_share_set_device_name($0) }
        NotificationCenter.default.addObserver(forName: AVAudioSession.interruptionNotification,
                                               object: nil, queue: .main) { [weak self] _ in
            self?.resumeAudio()
        }
        start()
        timer = Timer.scheduledTimer(withTimeInterval: 1, repeats: true) { [weak self] _ in
            self?.refresh()
        }
        RunLoop.main.add(timer!, forMode: .common)
        refresh()
    }

    func start() {
        if wifi_share_start(port) == 0 {
            startKeepAlive()
            UIApplication.shared.isIdleTimerDisabled = true
        }
        refresh()
    }

    func stop() {
        wifi_share_stop()
        player?.stop()
        UIApplication.shared.isIdleTimerDisabled = false
        refresh()
    }

    func toggle() {
        if wifi_share_running() != 0 { stop() } else { start() }
        refresh()
    }

    func locationManagerDidChangeAuthorization(_ manager: CLLocationManager) {
        tick = 0
        refresh()
    }

    func refresh() {
        let addr = Model.wifiIP()
        connected = addr != nil
        ip = addr ?? "—"
        running = wifi_share_running() != 0
        active = Int(wifi_share_active())
        linked = wifi_share_linked() != 0
        var eb = [CChar](repeating: 0, count: 256)
        _ = wifi_share_last_error(&eb, eb.count)
        error = running ? "" : String(cString: eb)
        var buf = [CChar](repeating: 0, count: 128)
        pcName = (wifi_share_pc_name(&buf, buf.count) > 0) ? String(cString: buf) : "—"
        if running, let p = player, !p.isPlaying { resumeAudio() }
        if connected {
            if tick % 5 == 0 {
                NEHotspotNetwork.fetchCurrent { [weak self] net in
                    DispatchQueue.main.async { self?.ssid = net?.ssid ?? "نامشخص" }
                }
            }
        } else {
            ssid = "—"
        }
        tick += 1
    }

    // Silent looping audio keeps the app (and the proxy) alive with the screen off.
    private func startKeepAlive() {
        do {
            try AVAudioSession.sharedInstance().setCategory(.playback, options: [.mixWithOthers])
            try AVAudioSession.sharedInstance().setActive(true)
            let p = try AVAudioPlayer(data: Model.silentWav())
            p.numberOfLoops = -1
            p.volume = 1.0
            p.play()
            player = p
        } catch {}
    }

    private func resumeAudio() {
        guard running else { return }
        try? AVAudioSession.sharedInstance().setActive(true)
        if let p = player { p.play() } else { startKeepAlive() }
    }

    static func silentWav() -> Data {
        let sr = 8000, n = 8000
        var d = Data()
        func u32(_ v: UInt32) { var x = v.littleEndian; d.append(Data(bytes: &x, count: 4)) }
        func u16(_ v: UInt16) { var x = v.littleEndian; d.append(Data(bytes: &x, count: 2)) }
        d.append("RIFF".data(using: .ascii)!); u32(UInt32(36 + n * 2))
        d.append("WAVE".data(using: .ascii)!)
        d.append("fmt ".data(using: .ascii)!); u32(16); u16(1); u16(1)
        u32(UInt32(sr)); u32(UInt32(sr * 2)); u16(2); u16(16)
        d.append("data".data(using: .ascii)!); u32(UInt32(n * 2))
        d.append(Data(count: n * 2))
        return d
    }

    static func wifiIP() -> String? {
        var head: UnsafeMutablePointer<ifaddrs>?
        guard getifaddrs(&head) == 0, let first = head else { return nil }
        defer { freeifaddrs(head) }
        var p: UnsafeMutablePointer<ifaddrs>? = first
        while let cur = p {
            let i = cur.pointee
            if let a = i.ifa_addr,
               a.pointee.sa_family == UInt8(AF_INET),
               String(cString: i.ifa_name) == "en0",
               (i.ifa_flags & UInt32(IFF_UP)) != 0 {
                var host = [CChar](repeating: 0, count: Int(NI_MAXHOST))
                getnameinfo(a, socklen_t(a.pointee.sa_len), &host, socklen_t(host.count), nil, 0, NI_NUMERICHOST)
                let s = String(cString: host)
                if !s.hasPrefix("169.254") { return s }
            }
            p = i.ifa_next
        }
        return nil
    }
}
