# 流式传输设计方案

文件 Network/stream_protocol.rs
定义流式传输协议，流帧格式

在node.rs当中修改：
- 增加behavior
- 在Node Command里面增加SendFileStream，默认已经接收到了对方的接收文件确认信息，直接建立流式传输，开始传输文件
- 在NetworkEvent里面增加FileStreamReceived，接收文件
- 流处理逻辑，按照发送方打开流，写入分块数据 → 接收方边收边写磁盘的方式进行。

其他要修改的内容：
protocol.rs改名为data_protocol.rs



