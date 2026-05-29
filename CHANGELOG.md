# Changelog

## [0.3.0](https://github.com/korund/notemill-worker/compare/v0.2.0...v0.3.0) (2026-05-29)


### Features

* **models:** support SenseVoice, Canary and Cohere engines, expand catalog ([900f85d](https://github.com/korund/notemill-worker/commit/900f85d141806892c5bad67fed8f29b2e144b154))


### Bug Fixes

* **config:** materialize audio defaults when section omitted ([ad1ab53](https://github.com/korund/notemill-worker/commit/ad1ab5360ddc5e7debb81e2f4185ebcfebf1af24))

## [0.2.0](https://github.com/korund/notemill-worker/compare/v0.1.0...v0.2.0) (2026-05-21)


### Features

* **preprocess:** add silero VAD before transcription ([c4947bc](https://github.com/korund/notemill-worker/commit/c4947bceede3d5500c2bc63fbd8c6a3651d0ea32))
* **preprocess:** chunk VAD segments before transcription ([1c00784](https://github.com/korund/notemill-worker/commit/1c00784d80fca5ff9e0ad92d7d053e7b885cd63a))
* **preprocess:** classify VAD output into Speech verdict and surface NoSpeech from pipeline ([b569404](https://github.com/korund/notemill-worker/commit/b5694040d6b9efee11693e60b0af86b5aee70340))
* **preprocess:** error-log when silero produces no signal at all (max_prob &lt; 0.1) ([3fb419d](https://github.com/korund/notemill-worker/commit/3fb419d7bd224c4d5ce7a5026c9c1651fbd01113))
* **queue:** asymmetric wire compatibility with producer-ahead detection ([3c6a1d1](https://github.com/korund/notemill-worker/commit/3c6a1d12a956f8d2fc8570496e23abcfc6054920))
* **queue:** emit NoSpeech NotifyResult when segmenter reports silent input ([038911b](https://github.com/korund/notemill-worker/commit/038911b7298e1c496d3417f2e59f0bbe99c5c08a))


### Performance Improvements

* **vad:** avoid trace Vec allocation when trace disabled ([ff92617](https://github.com/korund/notemill-worker/commit/ff92617102f84bea1e8a6c359ba49b717d60f03c))
