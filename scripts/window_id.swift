// Prints the window number of a process's largest normal window, for `screencapture -l`.
// Usage: swift scripts/window_id.swift <pid>
import CoreGraphics
import Foundation

guard CommandLine.arguments.count > 1, let pid = Int(CommandLine.arguments[1]) else {
    FileHandle.standardError.write("usage: window_id <pid>\n".data(using: .utf8)!)
    exit(2)
}
let list = CGWindowListCopyWindowInfo([.optionAll], kCGNullWindowID) as? [[String: Any]] ?? []
var best: (Int, Double)? = nil
for w in list {
    guard (w[kCGWindowOwnerPID as String] as? Int) == pid,
          (w[kCGWindowLayer as String] as? Int) == 0,
          let b = w[kCGWindowBounds as String] as? [String: Any],
          let width = b["Width"] as? Double, let height = b["Height"] as? Double,
          let id = w[kCGWindowNumber as String] as? Int else { continue }
    if best == nil || width * height > best!.1 { best = (id, width * height) }
}
guard let (id, _) = best else { exit(1) }
print(id)
