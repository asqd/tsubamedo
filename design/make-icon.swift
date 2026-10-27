// Draws the TsubameDo app icon: a swallow (tsubame) in flight on a night-blue tile.
// The bird is drawn on a 24-unit grid, y down, like the in-app logo (src/components/Logo.tsx).
// Usage: swift design/make-icon.swift <output.png>   (1024 × 1024, macOS icon grid)
import AppKit

let size: CGFloat = 1024
let output = CommandLine.arguments.dropFirst().first ?? "icon_1024.png"

func color(_ hex: UInt32, _ alpha: CGFloat = 1) -> CGColor {
    CGColor(
        srgbRed: CGFloat((hex >> 16) & 0xFF) / 255,
        green: CGFloat((hex >> 8) & 0xFF) / 255,
        blue: CGFloat(hex & 0xFF) / 255,
        alpha: alpha
    )
}

let space = CGColorSpace(name: CGColorSpace.sRGB)!
let context = CGContext(
    data: nil, width: Int(size), height: Int(size), bitsPerComponent: 8, bytesPerRow: 0,
    space: space, bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue
)!

// Tile: 824 × 824 centred, as in Apple's macOS icon template, with a soft shadow.
let tile = CGRect(x: 100, y: 100, width: 824, height: 824)
let tilePath = CGPath(roundedRect: tile, cornerWidth: 185, cornerHeight: 185, transform: nil)
context.saveGState()
context.setShadow(offset: CGSize(width: 0, height: -10), blur: 28, color: color(0x000000, 0.35))
context.addPath(tilePath)
context.setFillColor(color(0x15204A))
context.fillPath()
context.restoreGState()

context.saveGState()
context.addPath(tilePath)
context.clip()
let gradient = CGGradient(colorsSpace: space, colors: [color(0x3157C4), color(0x111A3D)] as CFArray, locations: [0, 1])!
context.drawLinearGradient(gradient, start: CGPoint(x: tile.minX, y: tile.maxY), end: CGPoint(x: tile.maxX, y: tile.minY), options: [])
context.restoreGState()

let unit: CGFloat = 640 / 24
let origin = CGPoint(x: 192, y: 832)
func p(_ x: CGFloat, _ y: CGFloat) -> CGPoint { CGPoint(x: origin.x + x * unit, y: origin.y - y * unit) }

func fill(_ hex: UInt32, shadow: Bool = true, _ build: (CGMutablePath) -> Void) {
    let path = CGMutablePath()
    build(path)
    context.saveGState()
    if shadow {
        context.setShadow(offset: CGSize(width: 0, height: -8), blur: 18, color: color(0x000000, 0.35))
    }
    context.addPath(path)
    context.setFillColor(color(hex))
    context.fillPath()
    context.restoreGState()
}

// The swallow, flying up and to the right: beak, head, a long swept wing, the forked tail.
let bird = { (path: CGMutablePath) in
    path.move(to: p(22, 4.2))                                                    // beak
    path.addCurve(to: p(17.2, 4.6), control1: p(20.4, 3.4), control2: p(18.6, 3.5))  // crown
    path.addCurve(to: p(12.6, 8.4), control1: p(15.6, 5.8), control2: p(14.2, 7.2))  // back
    path.addCurve(to: p(1.5, 6.2), control1: p(9.0, 6.4), control2: p(4.8, 5.4))     // upper edge of the wing
    path.addCurve(to: p(10.4, 11.6), control1: p(4.6, 8.6), control2: p(7.6, 10.4))  // wing's trailing edge
    path.addCurve(to: p(2.2, 18.8), control1: p(7.6, 14.0), control2: p(4.8, 16.6))  // upper tail streamer
    path.addCurve(to: p(9.6, 15.0), control1: p(5.0, 17.6), control2: p(7.6, 16.2))  // fork
    path.addCurve(to: p(5.6, 21.4), control1: p(8.2, 17.2), control2: p(6.8, 19.4))  // lower tail streamer
    path.addCurve(to: p(15.6, 11.6), control1: p(9.6, 18.6), control2: p(13.2, 15.0)) // belly
    path.addCurve(to: p(19.2, 6.6), control1: p(17.2, 9.6), control2: p(18.0, 7.6))  // breast
    path.addCurve(to: p(22, 4.2), control1: p(20.2, 5.8), control2: p(21.2, 5.0))    // throat to beak
    path.closeSubpath()
}
fill(0xF4F6FB, bird)
// The red throat of a barn swallow.
fill(0xFF6A3D, shadow: false) { path in
    path.move(to: p(20.7, 5.3))
    path.addCurve(to: p(18.6, 5.9), control1: p(19.9, 5.4), control2: p(19.1, 5.5))
    path.addCurve(to: p(19.2, 6.6), control1: p(18.6, 6.2), control2: p(18.9, 6.5))
    path.addCurve(to: p(20.7, 5.3), control1: p(19.8, 6.1), control2: p(20.3, 5.6))
    path.closeSubpath()
}
// Eye.
fill(0x15204A, shadow: false) { path in
    path.addEllipse(in: CGRect(x: p(18.9, 0).x, y: p(0, 4.95).y, width: 0.7 * unit, height: 0.7 * unit))
}

let image = context.makeImage()!
let rep = NSBitmapImageRep(cgImage: image)
try! rep.representation(using: .png, properties: [:])!.write(to: URL(fileURLWithPath: output))
print("wrote \(output)")
