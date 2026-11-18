import SwiftUI

@main
struct WiFiShareApp: App {
    @StateObject private var model = Model()
    var body: some Scene {
        WindowGroup { ContentView(model: model) }
    }
}
