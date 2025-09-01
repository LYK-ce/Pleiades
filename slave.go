package main

import (
	"context"
	"fmt"
	"log"

	"github.com/libp2p/go-libp2p/core/network"
)

func ListenString(ctx context.Context) {
	my_host.SetStreamHandler("/chat/1.0.0", func(s network.Stream) {
		defer s.Close()
		buf := make([]byte, 1024)
		n, err := s.Read(buf)
		if err != nil {
			log.Printf("read stream error: %v", err)
			return
		}
		fmt.Printf("收到消息: %s\n", string(buf[:n]))
	})
}

func Listen_Command(ctx context.Context) {
	my_host.SetStreamHandler(COMMAND_PROTOCOL, func(s network.Stream) {
		defer s.Close()
		command_buf := make([]byte, 8)
		n, err := s.Read(command_buf)
		if err != nil {
			log.Printf("read stream error: %v", err)
			return
		}
		payload := make([]byte, n)
		copy(payload, command_buf[:n])
		command_queue <- payload
	})
}

func Process_Worker() {
	for data := range command_queue {
		fmt.Printf("Processing command: %d\n", data)
	}
}
