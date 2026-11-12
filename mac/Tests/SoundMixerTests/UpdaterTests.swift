import CryptoKit
import Foundation
import Testing
@testable import SoundMixer

struct UpdaterTests {
    @Test func versionComparisonIsNumeric() {
        #expect(Updater.compare("0.10.0", "0.9.9") == .orderedDescending)
        #expect(Updater.compare("0.0.2", "0.0.2") == .orderedSame)
        #expect(Updater.compare("0.3", "0.3.0") == .orderedSame, "a missing part counts as 0")
        #expect(Updater.compare("1.0.0", "1.0.1") == .orderedAscending)
        #expect(Updater.compare("garbage", "0.0.1") == .orderedAscending, "unparsable parts count as 0")
    }

    @Test func feedDecodes() throws {
        let info = try JSONDecoder().decode(UpdateInfo.self, from: Data("""
            {"version":"0.0.2","url":"https://example.com/SoundMixer-0.0.2.zip","signature":"abc",
             "notes":"Notes","minimumSystemVersion":"14.2"}
            """.utf8))
        #expect(info.version == "0.0.2")
        #expect(info.url.lastPathComponent == "SoundMixer-0.0.2.zip")
        #expect(info.minimumSystemVersion == "14.2")
        // notes and minimum are optional
        _ = try JSONDecoder().decode(UpdateInfo.self, from: Data(#"{"version":"1","url":"https://e.com/a.zip","signature":""}"#.utf8))
    }

    @Test @MainActor func unconfiguredBuildNeverGoesOnline() async {
        // the test bundle has no feed URL or public key
        let u = Updater(bundle: Bundle(for: BundleToken.self))
        #expect(!u.isConfigured)
        await u.check(userInitiated: true)
        if case .failed = u.phase {} else { Issue.record("expected a 'not set up' failure, got \(u.phase)") }
    }

    /// The release key in mac/Updates.json is a valid Ed25519 public key.
    @Test func publishedPublicKeyIsValid() throws {
        let url = URL(fileURLWithPath: #filePath).deletingLastPathComponent().appendingPathComponent("../../Updates.json")
        let json = try JSONSerialization.jsonObject(with: Data(contentsOf: url)) as? [String: String]
        let key = try #require(json?["publicKey"].flatMap { Data(base64Encoded: $0) })
        _ = try Curve25519.Signing.PublicKey(rawRepresentation: key)
        #expect(json?["feedURL"]?.hasSuffix("/releases/latest/download/appcast.json") == true)
    }
}

private final class BundleToken {}
