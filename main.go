package main

//先解决有没有的问题，再解决好不好用的问题
import (
	"context"
	"flag"
	"fmt"
	"log"

	"github.com/libp2p/go-libp2p"
)

func main() {
	//初始化逻辑
	var work_mode int = -1 // -1 未设置，0 Master，1 Slave
	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	var err error
	var status bool

	//解析命令行参数，确定运行模式
	//Master mode 和 Slave mode
	var mode string
	flag.StringVar(&mode, "mode", "slave", "Specify mode: master or slave")
	flag.Parse()
	switch mode {
	case "master":
		work_mode = 0
		fmt.Println("Running in Master mode")
	case "slave":
		work_mode = 1
		fmt.Println("Running in Slave mode")
	default:
		fmt.Println("Please specify mode: -mode=master or -mode=slave")
		return
	}
	fmt.Printf("Work mode: %d\n", work_mode)

	status = Initialize_WorkSpace()
	if !status {
		log.Fatalf("Initialize_WorkSpace error")
		return
	}
	//新建 libp2p Host, 监听端口
	my_host, err = libp2p.New(libp2p.ListenAddrStrings("/ip4/0.0.0.0/tcp/0"))
	if err != nil {
		log.Fatalf("libp2p.New error: %v", err)
	}
	defer my_host.Close()

	err = Setup_MDNS(my_host)
	if err != nil {
		log.Fatalf("Setup_MDNS error: %v", err)
	}

	//退出routine控制
	go Handle_Signal(cancel) // 信号 goroutine
	go Process_Worker()      // 命令处理 goroutine

	switch work_mode {
	case 0:
		//Master模式，主动发送命令与文件
		//读取命令并解析命令，然后向Slave节点发送等内容
		//解析命令，发送主要内容等
		go Broadcast(ctx)

	case 1:
		//Slave模式，被动等待命令与接收即可
		// go ListenString(ctx) // 监听字符串消息
		Listen_Command(ctx)

	default:
		log.Fatalf("Invalid work mode: %d", work_mode)
	}
	//

	<-ctx.Done() // 主 goroutine 阻塞直到 cancel 被调用
	//退出阶段，清理工作空间
	Cleanup_WorkSpace()
	fmt.Println("bye")
}
