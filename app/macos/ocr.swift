// TiLex OCR sidecar using Apple Vision framework.
// Note: We deliberately omit Easydict's layout reconstruction and line merging
// (OCRBandMerger, OCRSectionMerger, OCRPoetryDetector) because TiLex only
// requires plain recognized text lines.

import AppKit
import Foundation
import Vision

let args = CommandLine.arguments
guard args.count >= 2 else {
    FileHandle.standardError.write("usage: tilex-ocr <image>\n".data(using: .utf8)!)
    exit(2)
}
guard let image = NSImage(contentsOfFile: args[1]),
      let cgImage = image.cgImage(forProposedRect: nil, context: nil, hints: nil) else {
    FileHandle.standardError.write("cannot read image\n".data(using: .utf8)!)
    exit(3)
}

let request = VNRecognizeTextRequest()
request.recognitionLevel = .accurate
request.usesLanguageCorrection = true
// Vision 的默认 recognitionLanguages 只有 ["en-US"]，不显式设中文就一个汉字都认不出来。
request.recognitionLanguages = ["zh-Hans", "zh-Hant", "en-US", "ja-JP"]

let handler = VNImageRequestHandler(cgImage: cgImage, options: [:])
do {
    try handler.perform([request])
} catch {
    FileHandle.standardError.write("vision failed: \(error)\n".data(using: .utf8)!)
    exit(4)
}

// 显式向下转型，别依赖 VNRecognizeTextRequest 覆盖过的类型化 results：
// 两种 SDK 形状下这么写都编得过（多余时只是一条 "always succeeds" 警告），
// 而少写这个 cast 在 results 为 [VNObservation]? 的 SDK 上是编译错误。
// Easydict 的 AppleOCREngine.swift:246 也是这么写的。
let observations = (request.results as? [VNRecognizedTextObservation]) ?? []
let lines = observations.compactMap { $0.topCandidates(1).first?.string }
print(lines.joined(separator: "\n"))
