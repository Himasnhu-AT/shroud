import AppKit
// Usage: swift make_cover.swift screenshots/01_squad_close.png cover.png
let a = CommandLine.arguments
let src = NSImage(contentsOfFile: a[1])!
var rect = CGRect(x: 0, y: 0, width: 0, height: 0)
let cg = src.cgImage(forProposedRect: &rect, context: nil, hints: nil)!
let W = 630, H = 500
// Crop a 630:500 window around the squad (left-centre of the frame), from the 1920x1080 shot.
let ch = 880.0, cw = ch * Double(W) / Double(H)
let cx = 1010.0 - cw / 2, cy = 150.0
let crop = cg.cropping(to: CGRect(x: cx, y: cy, width: cw, height: ch))!
let rep = NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: W, pixelsHigh: H, bitsPerSample: 8, samplesPerPixel: 4, hasAlpha: true, isPlanar: false, colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0)!
NSGraphicsContext.saveGraphicsState()
NSGraphicsContext.current = NSGraphicsContext(bitmapImageRep: rep)
let ctx = NSGraphicsContext.current!.cgContext
ctx.interpolationQuality = .high
ctx.draw(crop, in: CGRect(x: 0, y: 0, width: W, height: H))
// Dark gradient at the bottom for the title.
let grad = CGGradient(colorsSpace: CGColorSpaceCreateDeviceRGB(), colors: [NSColor(white: 0, alpha: 0.0).cgColor, NSColor(white: 0, alpha: 0.8).cgColor] as CFArray, locations: [0, 1])!
ctx.drawLinearGradient(grad, start: CGPoint(x: 0, y: 220), end: CGPoint(x: 0, y: 0), options: [])
let shadow = NSShadow(); shadow.shadowColor = NSColor(white: 0, alpha: 0.7); shadow.shadowBlurRadius = 6; shadow.shadowOffset = NSSize(width: 0, height: -2)
func draw(_ s: String, _ font: NSFont, _ color: NSColor, _ y: CGFloat, kern: CGFloat) {
    let attrs: [NSAttributedString.Key: Any] = [.font: font, .foregroundColor: color, .kern: kern, .shadow: shadow]
    NSAttributedString(string: s, attributes: attrs).draw(at: NSPoint(x: 34, y: y))
}
let barlow = NSFont(name: "Barlow-Medium", size: 82) ?? NSFont.systemFont(ofSize: 82, weight: .heavy)
// Title: the largest size up to 86pt that fits the cover width; the eyebrow sits just above it.
let title = "PATTERN OF LIFE", titleKern: CGFloat = 2
var titleFont = NSFont.systemFont(ofSize: 86, weight: .black)
while titleFont.pointSize > 30 && NSAttributedString(string: title, attributes: [.font: titleFont, .kern: titleKern]).size().width > CGFloat(W) - 68 {
    titleFont = NSFont.systemFont(ofSize: titleFont.pointSize - 1, weight: .black)
}
let titleY: CGFloat = 40
draw("ADAPTIVE SQUAD SHOOTER", NSFont.systemFont(ofSize: 15, weight: .bold), NSColor(red: 0.98, green: 0.45, blue: 0.35, alpha: 1), titleY + titleFont.ascender + 10, kern: 3)
draw(title, titleFont, .white, titleY, kern: titleKern)
draw("They learn from you.", NSFont.systemFont(ofSize: 17, weight: .medium), NSColor(white: 0.9, alpha: 1), 22 - 4, kern: 0.5)
NSGraphicsContext.restoreGraphicsState()
try! rep.representation(using: .png, properties: [:])!.write(to: URL(fileURLWithPath: a[2]))
