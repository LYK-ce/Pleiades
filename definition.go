package main

import (
	"container/list"
	"os"
	"sync"

	"github.com/libp2p/go-libp2p/core/host"
	"github.com/libp2p/go-libp2p/core/peer"
)

const (
	WORKSPACE_NAME = "Pleiades_workspace"
	LOG_NAME       = "Pleiades_work_log.log"

	SERVICE_TAG      = "Pleiades_tcp"
	COMMAND_PROTOCOL = "/cmd/1.0.0"
	FILE_PROTOCOL    = "/file/1.0.0"
	DATA_PROTOCOL    = "/data/1.0.0"

	CMD_FILE   = 0x1
	CMD_DATA   = 0x2
	CMD_EXEC   = 0x3
	CMD_RESULT = 0x4
	RES_FILE   = 0x5
	RES_DATA   = 0x6
	RES_EXEC   = 0x7
	RES_RESULT = 0x8
	//传递数据时需要注意考虑大小端的问题

	//控制命令队列的长度
	COMMAND_QUEUE_SIZE = 100
)

var task_list = list.New()

// 全局日志文件变量
var log_file *os.File
var log_mutex sync.Mutex

var seen = map[peer.ID]struct{}{}

// 一个简单的 Notifee，发现节点时回调
type notifee struct{ h host.Host }

// 一些全局变量
var my_host host.Host

var command_queue = make(chan []byte, COMMAND_QUEUE_SIZE)
