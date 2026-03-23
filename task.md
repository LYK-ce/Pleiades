1. 在config.toml当中有[Runtime] deive = "cpu"
 你现在重新给我实现runtime.rs里面的Init ONNX，根据config里面的这一条进行判断，用match的方式来选择设备。原来那种实现方案不要了
 完成
2. 根据runtime test.rs里最后添加的函数注释，补全对应内容