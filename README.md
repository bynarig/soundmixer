# SoundMixer

A volume knob for every app on your Mac.

SoundMixer lets you make one app quieter without touching the others. For example, you can keep your music low while a video call stays loud. You can also send an app to a different speaker or headphones, or give it a sound boost or an equaliser.

## What you need

- A Mac with **macOS 14.2 or newer**. On **macOS 26** the window gets Apple's new "Liquid Glass" look.
- The first time you change an app's sound, macOS may ask whether SoundMixer can record audio from other apps. Click **Allow**. SoundMixer needs this permission to adjust other apps' sound. It does not record or save anything.

## Starting it

1. Download **SoundMixer-…-macos-arm64.zip** from the [Releases page](https://github.com/bynarig/soundmixer/releases) (Apple silicon Macs).
2. Double-click the zip and drag **SoundMixer** to your Applications folder.
3. Open it. The first time, macOS says it can't check the app, because it isn't notarized by Apple. Click **Done**, then open **System Settings › Privacy & Security**, scroll down and click **Open Anyway**.

To build it yourself, open **Terminal** in this folder and type:

```
mac/build-app.sh
open mac/build/SoundMixer.app
```

(This needs Xcode from the App Store and the free Rust tools from https://rustup.rs.)

## Using it

The window looks and works like System Settings.

**Output.** This is where sound comes out.
- **Device:** choose your speakers or headphones.
- **Volume:** the main volume. Click the speaker to mute everything.
- **Balance:** more sound on the left or the right. This only appears if your device supports it.
- **Boost:** makes quiet things louder without distortion.
- **Headphone EQ:** click **Choose File…** and pick a correction profile for your headphones. You can find these on the AutoEQ website, saved as "ParametricEQ.txt".
- **Equalizer:** switch it on, then click **Edit…** to shape the bass and treble yourself or pick a preset.

**Input.** Choose your microphone and set its level.

**Sound Effects.** Choose which speaker plays alert sounds.

**Applications.** Every app that is playing sound appears here, with its icon.
- Drag an app's **slider** to change its volume. The first three quarters go up to 100 %. Past that, the slider turns orange and goes up to 400 % (louder than normal).
- Double-click the **percentage** to reset an app to 100 %.
- The **speaker button** mutes just that app.
- The **sliders button** on the right opens more options:
  - **Output:** which speaker to use.
  - **Balance** and **Boost**.
  - **Equalizer**.
  - **Always Show:** keeps the app in the list even when it is quiet.
- Right-click an app for quick actions.

**Other settings.**
- The **circular arrow** at the top right resets every app to normal.
- **SoundMixer › Settings…** (⌘,) has **Latency**. Leave it on **Automatic**. If you hear crackles, choose **Glitch-proof**.

## Good to know

- Apps you never touch play exactly as before. SoundMixer only steps in for apps you have changed.
- Your settings are saved by themselves and come back the next time you open the app.
- If you unplug the headphones an app was using, its sound moves to your normal speakers.
- The app follows your Mac's light or dark mode. It also follows the Accessibility display settings: Reduce Transparency, Increase Contrast, Reduce Motion and Differentiate Without Colour. Everything works with VoiceOver and the keyboard.

## If something goes wrong

- **No sound from an app:** check that the app isn't muted (red speaker) and that its volume isn't at 0 %. If that doesn't help, click the circular arrow at the top right to reset everything.
- **Crackling:** open **SoundMixer › Settings…** and set **Latency** to **Glitch-proof**.
- **Start fresh:** quit SoundMixer and delete the settings file in the `~/Library/Application Support/SoundMixer/` folder. To find it, press Shift-Command-G in Finder and paste that path.

## License

MIT. See [LICENSE](LICENSE).
