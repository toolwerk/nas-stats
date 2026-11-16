// Draws the app icon: a NAS case with drive bays and a green status light (the dark StatusOk of the app) on
// a dark ground. Writes every size the asset catalog lists into
// AppIcon.appiconset.
//
//   swift scripts/icon/make-icon.swift
import CoreGraphics
import Foundation
import ImageIO
import UniformTypeIdentifiers

let root = URL(fileURLWithPath: #filePath).deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
let out = root.appendingPathComponent("apps/NasStats/NasStats/Assets.xcassets/AppIcon.appiconset")
try FileManager.default.createDirectory(at: out, withIntermediateDirectories: true)

func color(_ hex: UInt32, _ alpha: CGFloat = 1) -> CGColor {
    CGColor(red: CGFloat((hex >> 16) & 0xFF) / 255, green: CGFloat((hex >> 8) & 0xFF) / 255,
            blue: CGFloat(hex & 0xFF) / 255, alpha: alpha)
}

func rounded(_ rect: CGRect, _ radius: CGFloat) -> CGPath {
    CGPath(roundedRect: rect, cornerWidth: radius, cornerHeight: radius, transform: nil)
}

/// `macOS` bakes the rounded square in; iOS gets a full-bleed square without
/// an alpha channel (App Store Connect rejects an iOS icon with one).
func draw(size: Int, macOS: Bool) -> CGImage? {
    let s = CGFloat(size)
    let alpha: CGImageAlphaInfo = macOS ? .premultipliedLast : .noneSkipLast
    guard let ctx = CGContext(data: nil, width: size, height: size, bitsPerComponent: 8, bytesPerRow: 0,
                              space: CGColorSpace(name: CGColorSpace.sRGB)!,
                              bitmapInfo: alpha.rawValue) else { return nil }
    let rect = CGRect(x: 0, y: 0, width: s, height: s)
    if macOS {
        ctx.addPath(rounded(rect.insetBy(dx: s * 0.05, dy: s * 0.05), s * 0.2))
        ctx.clip()
    }
    // Ground: near-black with a faint blue cast, lighter towards the top.
    let ground = CGGradient(colorsSpace: CGColorSpace(name: CGColorSpace.sRGB),
                            colors: [color(0x1B2130), color(0x0C0E12)] as CFArray, locations: [0, 1])!
    ctx.drawLinearGradient(ground, start: CGPoint(x: 0, y: s), end: CGPoint(x: 0, y: 0), options: [])

    // The case.
    let caseRect = CGRect(x: s * 0.22, y: s * 0.20, width: s * 0.56, height: s * 0.60)
    ctx.addPath(rounded(caseRect, s * 0.05))
    ctx.setFillColor(color(0x2A3142))
    ctx.fillPath()
    ctx.addPath(rounded(caseRect, s * 0.05))
    ctx.setStrokeColor(color(0xFFFFFF, 0.14))
    ctx.setLineWidth(s * 0.008)
    ctx.strokePath()

    // Four drive bays, a small handle on each.
    let inset = s * 0.04
    let bayGap = s * 0.025
    let bayH = (caseRect.height - 2 * inset - s * 0.09 - 3 * bayGap) / 4
    for i in 0..<4 {
        let y = caseRect.maxY - inset - bayH - CGFloat(i) * (bayH + bayGap)
        let bay = CGRect(x: caseRect.minX + inset, y: y, width: caseRect.width - 2 * inset, height: bayH)
        ctx.addPath(rounded(bay, s * 0.02))
        ctx.setFillColor(color(0x161A24))
        ctx.fillPath()
        let handle = CGRect(x: bay.maxX - s * 0.11, y: bay.midY - s * 0.008, width: s * 0.08, height: s * 0.016)
        ctx.addPath(rounded(handle, s * 0.008))
        ctx.setFillColor(color(0xFFFFFF, 0.25))
        ctx.fillPath()
    }

    // The status light in the foot of the case, with a soft glow.
    let light = CGPoint(x: caseRect.minX + inset + s * 0.03, y: caseRect.minY + inset + s * 0.02)
    let glow = CGGradient(colorsSpace: CGColorSpace(name: CGColorSpace.sRGB),
                          colors: [color(0x3FCB7A, 0.55), color(0x3FCB7A, 0)] as CFArray, locations: [0, 1])!
    ctx.drawRadialGradient(glow, startCenter: light, startRadius: 0, endCenter: light, endRadius: s * 0.07, options: [])
    ctx.setFillColor(color(0x3FCB7A))
    ctx.fillEllipse(in: CGRect(x: light.x - s * 0.018, y: light.y - s * 0.018, width: s * 0.036, height: s * 0.036))
    return ctx.makeImage()
}

func write(_ image: CGImage, to url: URL) throws {
    guard let dest = CGImageDestinationCreateWithURL(url as CFURL, UTType.png.identifier as CFString, 1, nil) else {
        throw NSError(domain: "icon", code: 1)
    }
    CGImageDestinationAddImage(dest, image, nil)
    guard CGImageDestinationFinalize(dest) else { throw NSError(domain: "icon", code: 2) }
}

struct Entry { let idiom: String; let size: Int; let scale: Int; let platform: String? }
var entries: [Entry] = [Entry(idiom: "universal", size: 1024, scale: 1, platform: "ios")]
for size in [16, 32, 128, 256, 512] {
    for scale in [1, 2] { entries.append(Entry(idiom: "mac", size: size, scale: scale, platform: nil)) }
}
var images: [[String: String]] = []
for e in entries {
    let name = e.idiom == "mac" ? "mac-\(e.size)x\(e.size)@\(e.scale)x.png" : "ios-1024.png"
    guard let img = draw(size: e.size * e.scale, macOS: e.idiom == "mac") else { continue }
    try write(img, to: out.appendingPathComponent(name))
    // The single-size iOS icon carries a platform and no scale.
    var entry = ["idiom": e.idiom, "size": "\(e.size)x\(e.size)", "filename": name]
    if let p = e.platform { entry["platform"] = p } else { entry["scale"] = "\(e.scale)x" }
    images.append(entry)
}
let contents: [String: Any] = ["images": images, "info": ["author": "xcode", "version": 1]]
let data = try JSONSerialization.data(withJSONObject: contents, options: [.prettyPrinted, .sortedKeys])
try data.write(to: out.appendingPathComponent("Contents.json"))
print("icon: \(images.count) files in \(out.path)")
