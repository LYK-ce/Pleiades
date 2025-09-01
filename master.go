package main

import (
	"bufio"
	"context"
	"log"
	"os"
	"time"
)

// 广播输入文本到所有已发现的节点
func Broadcast(ctx context.Context) {
	scanner := bufio.NewScanner(os.Stdin)
	for scanner.Scan() {
		line := scanner.Text()
		if line == "" { // 空行跳过
			continue
		}

		// 遍历所有已发现节点
		for id := range seen {
			// 跳过自己
			if id == my_host.ID() {
				continue
			}
			sCtx, cancel := context.WithTimeout(ctx, 3*time.Second)

			// 建立流
			s, err := my_host.NewStream(sCtx, id, COMMAND_PROTOCOL)
			if err != nil {
				log.Printf("stream to %s failed: %v", id, err)
				cancel()
				continue
			}

			// 发送文本
			if _, wErr := s.Write([]byte{CMD_EXEC}); wErr != nil {
				log.Printf("send to %s failed: %v", id, wErr)
			}
			s.Close()
			cancel()
		}
	}
	if err := scanner.Err(); err != nil {
		log.Printf("stdin error: %v", err)
	}
}
