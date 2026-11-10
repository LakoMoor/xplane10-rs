import CoreGraphics
import Foundation
// Print "<id> <width>x<height> <name>" for every on-screen window owned by X-Plane.
let list = CGWindowListCopyWindowInfo([.optionOnScreenOnly], kCGNullWindowID) as? [[String: Any]] ?? []
for w in list {
    guard let owner = w[kCGWindowOwnerName as String] as? String, owner.contains("X-Plane") else { continue }
    let id = w[kCGWindowNumber as String] as? Int ?? 0
    let b = w[kCGWindowBounds as String] as? [String: Any] ?? [:]
    print(id, "\(b["Width"] ?? 0)x\(b["Height"] ?? 0)", w[kCGWindowName as String] as? String ?? "")
}
