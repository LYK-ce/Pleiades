1|start|2026-03-20T09:25:00Z|task=runtime_init_by_config_device
1|end|2026-03-20T09:26:49Z|dep=src/Config/config.toml[Runtime].deive|chg=src/main.rs,src/Runtime/runtime.rs
1|fix|2026-03-20T09:28:17Z|rm=cfg(feature/not(feature)) in Init_ONNX
1|fix|2026-03-20T09:29:56Z|sig=Init_ONNX(RuntimeInitConfig),rm=backend-candle/tract cfg
2|start|2026-03-21T05:37:00Z|task=fix_load_model_ep|dep=Init_ONNX
2|end|2026-03-23T02:59:00Z|chg=tests/common/runtime_test.rs|add=Test_Qwen3_Full_Model(),path=tests/test_models/qwen3-0.6b/onnx/model.onnx,token=[108386,6313],inputs=3,note=onnx_not_support_zero_dim
2|add|2026-03-23T02:35:00Z|file=tests/common/token_extract.py
2|add|2026-03-23T02:48:00Z|file=tests/common/check_onnx_input.py
