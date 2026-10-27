// Screenshots of the interface for the download page, from the dev bridge in WebKit (the
// engine of the app on macOS), at twice the size, so the page shows them sharp:
//
//   swift design/screenshot.swift <url> <out.png> <width> <height> <setup.js> <steps.js> [dark]
//
// setup.js runs once on the first load (localStorage: which collection to open), then the page
// loads again and steps.js runs: an async body that clicks through the interface.

import AppKit
import WebKit

let args = CommandLine.arguments
guard args.count >= 7, let url = URL(string: args[1]), let width = Double(args[3]), let height = Double(args[4]) else {
    FileHandle.standardError.write("usage: screenshot.swift <url> <out.png> <width> <height> <setup.js> <steps.js> [dark]\n".data(using: .utf8)!)
    exit(2)
}
let out = URL(fileURLWithPath: args[2])
let setup = try! String(contentsOfFile: args[5], encoding: .utf8)
let steps = try! String(contentsOfFile: args[6], encoding: .utf8)
let dark = args.count > 7 && args[7] == "dark"

final class Shooter: NSObject, WKNavigationDelegate {
    let web: WKWebView
    let window: NSWindow
    var loads = 0

    init(size: NSSize) {
        web = WKWebView(frame: NSRect(origin: .zero, size: size))
        window = NSWindow(contentRect: NSRect(origin: NSPoint(x: -20000, y: -20000), size: size), styleMask: [.borderless], backing: .buffered, defer: false)
        super.init()
        window.contentView = web
        window.appearance = NSAppearance(named: dark ? .darkAqua : .aqua)
        window.orderFrontRegardless()
        web.navigationDelegate = self
    }

    func webView(_ webView: WKWebView, didFinish navigation: WKNavigation!) {
        loads += 1
        if loads == 1 {
            web.evaluateJavaScript(setup) { _, _ in self.web.reload() }
            return
        }
        Task { @MainActor in
            // The interface fetches the collection first.
            try? await Task.sleep(nanoseconds: 1_500_000_000)
            do {
                _ = try await web.callAsyncJavaScript(steps, contentWorld: .page)
            } catch {
                FileHandle.standardError.write("steps: \(error)\n".data(using: .utf8)!)
                exit(1)
            }
            try? await Task.sleep(nanoseconds: 800_000_000)
            let config = WKSnapshotConfiguration()
            config.afterScreenUpdates = true
            web.takeSnapshot(with: config) { image, error in
                guard let image, let tiff = image.tiffRepresentation, let bitmap = NSBitmapImageRep(data: tiff),
                      let png = bitmap.representation(using: .png, properties: [:]) else {
                    FileHandle.standardError.write("snapshot: \(String(describing: error))\n".data(using: .utf8)!)
                    exit(1)
                }
                try! png.write(to: out)
                print("\(out.path): \(bitmap.pixelsWide)×\(bitmap.pixelsHigh)")
                exit(0)
            }
        }
    }
}

let app = NSApplication.shared
app.setActivationPolicy(.prohibited)
let shooter = Shooter(size: NSSize(width: width, height: height))
shooter.web.load(URLRequest(url: url))
DispatchQueue.main.asyncAfter(deadline: .now() + 60) {
    FileHandle.standardError.write("timed out\n".data(using: .utf8)!)
    exit(1)
}
app.run()
