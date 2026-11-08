// Renders the "Get more fonts" sample line in a font file, for the picker rows (a font that
// isn't installed can't draw its own preview). Writes <out>-dark.png and <out>-light.png at 2×.
//
//   swift scripts/font_samples.swift <Regular.ttf|otf> resources/font-samples/<id>
import AppKit
import CoreText

let args = CommandLine.arguments
guard args.count == 3 else { print("usage: font_samples.swift <font file> <out prefix>"); exit(1) }
let sample = "fn main() -> i32 { a != b && c => 0 }"
let (w, h, scale, size) = (320, 16, 2, 12.0)

guard let descs = CTFontManagerCreateFontDescriptorsFromURL(URL(fileURLWithPath: args[1]) as CFURL) as? [CTFontDescriptor],
      let desc = descs.first else { print("no font in \(args[1])"); exit(1) }
let font = CTFontCreateWithFontDescriptor(desc, CGFloat(size) * CGFloat(scale), nil)

// The text colors the rows use on dark and light backgrounds (secondary text).
for (suffix, gray) in [("dark", 0.70), ("light", 0.36)] {
    let cs = CGColorSpaceCreateDeviceRGB()
    let ctx = CGContext(data: nil, width: w * scale, height: h * scale, bitsPerComponent: 8, bytesPerRow: 0, space: cs,
                        bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)!
    let color = CGColor(red: gray, green: gray, blue: gray, alpha: 1)
    let attrs: [NSAttributedString.Key: Any] = [.font: font, .foregroundColor: color]
    let line = CTLineCreateWithAttributedString(NSAttributedString(string: sample, attributes: attrs))
    // Center the line box vertically: baseline at the descent plus half the spare room.
    let ascent = CTFontGetAscent(font), descent = CTFontGetDescent(font)
    let y = (CGFloat(h * scale) - (ascent + descent)) / 2 + descent
    ctx.textPosition = CGPoint(x: 0, y: y)
    CTLineDraw(line, ctx)
    let rep = NSBitmapImageRep(cgImage: ctx.makeImage()!)
    try! rep.representation(using: .png, properties: [:])!.write(to: URL(fileURLWithPath: "\(args[2])-\(suffix).png"))
}
