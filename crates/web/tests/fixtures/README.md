# Synthetic media fixture

`phone-h264.mp4` is one second of FFmpeg `testsrc2`, no audio or real camera data.
Generate headlessly (no preview/player):

```powershell
ffmpeg -f lavfi -i 'testsrc2=size=320x180:rate=15' -t 1 -an -c:v libx264 -profile:v baseline -level 3.0 -pix_fmt yuv420p -movflags +faststart phone-h264.mp4
```

This tests file decode and `playsinline`, not WebRTC, hardware decode, or iOS.
