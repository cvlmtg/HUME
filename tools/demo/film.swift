// Usage: film <pid> <output.mov>
// Films the on-screen window of process <pid> with ScreenCaptureKit into a
// lossless QuickTime Animation film, until SIGINT. Prints "filming" once the
// first frame is in. The film has no alpha channel, so the window's rounded
// corners come out black: given transparent pixels, ffmpeg's GIF encoder
// stores each frame nearly whole.
//
// ScreenCaptureKit delivers a frame only when the window changes, so a writer
// thread repeats the latest frame to keep a constant 25 fps timeline. The film
// is lossless because the GIF encoder stores only the pixels that differ from
// the previous frame, and a lossy codec makes every frame differ.
import AppKit
import CoreMedia
import CoreVideo
import Foundation
import ScreenCaptureKit

let fps = 25

func fail(_ message: String) -> Never {
    FileHandle.standardError.write("film: \(message)\n".data(using: .utf8)!)
    exit(1)
}

guard CommandLine.arguments.count == 3, let pid = Int32(CommandLine.arguments[1]) else {
    fail("usage: film <pid> <output.mov>")
}
let output = CommandLine.arguments[2]

// The lock guards latest and end, the only state that changes after init.
final class Frames: NSObject, SCStreamOutput, SCStreamDelegate, @unchecked Sendable {
    let width: Int
    let height: Int
    private let lock = NSLock()
    private var latest: Data?
    private var end: Date?

    init(width: Int, height: Int) {
        self.width = width
        self.height = height
    }

    var current: Data? {
        lock.lock()
        defer { lock.unlock() }
        return latest
    }

    // The instant filming stops: the film's last frame is the one due then.
    var stoppedAt: Date? {
        get {
            lock.lock()
            defer { lock.unlock() }
            return end
        }
        set {
            lock.lock()
            end = newValue
            lock.unlock()
        }
    }

    func stream(_ stream: SCStream, didOutputSampleBuffer sampleBuffer: CMSampleBuffer, of type: SCStreamOutputType) {
        guard
            let attachments = CMSampleBufferGetSampleAttachmentsArray(sampleBuffer, createIfNecessary: false)
                as? [[SCStreamFrameInfo: Any]],
            let raw = attachments.first?[.status] as? Int,
            SCFrameStatus(rawValue: raw) == .complete,
            let pixels = CMSampleBufferGetImageBuffer(sampleBuffer)
        else { return }
        guard CVPixelBufferGetWidth(pixels) == width, CVPixelBufferGetHeight(pixels) == height else {
            fail("the window changed size while filming")
        }
        CVPixelBufferLockBaseAddress(pixels, .readOnly)
        defer { CVPixelBufferUnlockBaseAddress(pixels, .readOnly) }
        guard let base = CVPixelBufferGetBaseAddress(pixels) else { fail("a frame has no pixels") }
        // Rows may be padded past width * 4 bytes; the film's rows are not.
        let stride = CVPixelBufferGetBytesPerRow(pixels)
        let row = width * 4
        var frame = Data(count: row * height)
        frame.withUnsafeMutableBytes { dst in
            for y in 0..<height {
                dst.baseAddress!.advanced(by: y * row).copyMemory(from: base.advanced(by: y * stride), byteCount: row)
            }
        }
        lock.lock()
        latest = frame
        lock.unlock()
    }

    func stream(_ stream: SCStream, didStopWithError error: Error) {
        fail("filming stopped: \(error.localizedDescription)")
    }
}

func start() async throws {
    let content = try await SCShareableContent.excludingDesktopWindows(true, onScreenWindowsOnly: true)
    guard let window = content.windows.first(where: { $0.owningApplication?.processID == pid && $0.windowLayer == 0 })
    else { fail("no on-screen window for pid \(pid)") }

    let filter = SCContentFilter(desktopIndependentWindow: window)
    let scale = CGFloat(filter.pointPixelScale)
    let frames = Frames(width: Int(filter.contentRect.width * scale), height: Int(filter.contentRect.height * scale))

    let config = SCStreamConfiguration()
    config.width = frames.width
    config.height = frames.height
    config.pixelFormat = kCVPixelFormatType_32BGRA
    config.minimumFrameInterval = CMTime(value: 1, timescale: CMTimeScale(fps))
    config.showsCursor = false
    // The terminal's colours are sRGB; the display's own space would shift them.
    config.colorSpaceName = CGColorSpace.sRGB

    let ffmpeg = Process()
    ffmpeg.executableURL = URL(fileURLWithPath: "/usr/bin/env")
    ffmpeg.arguments = [
        "ffmpeg", "-v", "error", "-y",
        "-f", "rawvideo", "-pixel_format", "bgra", "-video_size", "\(frames.width)x\(frames.height)",
        "-framerate", "\(fps)", "-i", "-",
        "-c:v", "qtrle", "-pix_fmt", "rgb24", output,
    ]
    // At a lower priority macOS may run ffmpeg on its efficiency cores, which
    // do not keep up with the film.
    ffmpeg.qualityOfService = .userInteractive
    let input = Pipe()
    ffmpeg.standardInput = input
    try ffmpeg.run()

    let stream = SCStream(filter: filter, configuration: config, delegate: frames)
    try stream.addStreamOutput(frames, type: .screen, sampleHandlerQueue: DispatchQueue(label: "film.frames"))
    try await stream.startCapture()

    while frames.current == nil { try await Task.sleep(nanoseconds: 10_000_000) }
    print("filming")
    fflush(stdout)

    // Writes as many frames as the wall clock says are due, so the film keeps
    // real time even when ffmpeg falls behind for a moment.
    let writer = Thread {
        let began = Date()
        var written = 0
        while true {
            let end = frames.stoppedAt
            let due = Int((end ?? Date()).timeIntervalSince(began) * Double(fps)) + 1
            if let frame = frames.current {
                while written < due {
                    input.fileHandleForWriting.write(frame)
                    written += 1
                }
            }
            if end != nil { break }
            Thread.sleep(forTimeInterval: 1.0 / Double(fps) / 4)
        }
        do {
            try input.fileHandleForWriting.close()
        } catch {
            fail("could not close ffmpeg's input: \(error.localizedDescription)")
        }
    }
    writer.qualityOfService = .userInteractive
    writer.start()

    interrupt.setEventHandler {
        frames.stoppedAt = Date()
        Task {
            do {
                try await stream.stopCapture()
            } catch {
                fail("could not stop filming: \(error.localizedDescription)")
            }
            ffmpeg.waitUntilExit()
            exit(ffmpeg.terminationStatus)
        }
    }
    interrupt.resume()
}

signal(SIGINT, SIG_IGN)
let interrupt = DispatchSource.makeSignalSource(signal: SIGINT, queue: .main)

// ScreenCaptureKit asserts unless the process has a window server connection,
// which a command-line tool opens through NSApplication.
_ = NSApplication.shared

Task {
    do {
        try await start()
    } catch {
        fail(error.localizedDescription)
    }
}
dispatchMain()
