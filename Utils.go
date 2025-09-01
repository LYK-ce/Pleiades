// signal.go
package main

import (
	"bufio"
	"context"
	"fmt"
	"os"
	"os/signal"
	"syscall"
	"time"
)

// HandleSignal 捕获 Ctrl-C / SIGTERM，然后调用 cancel 通知退出
func Handle_Signal(cancel context.CancelFunc) {
	sig := make(chan os.Signal, 1)
	signal.Notify(sig, syscall.SIGINT, syscall.SIGTERM)
	<-sig
	fmt.Println("\n收到退出信号，开始清理...")
	cancel()
}

func Scan_Input(ctx context.Context) {
	scanner := bufio.NewScanner(os.Stdin)
	for {
		if !scanner.Scan() { // 遇到 EOF 直接结束本 goroutine
			return
		}
		line := scanner.Text()
		fmt.Println("你输入的是:", line)
	}
}

func Initialize_WorkSpace() bool {
	folderName := WORKSPACE_NAME
	if _, err := os.Stat(folderName); err == nil {
		return false
	}
	err := os.Mkdir(folderName, 0755)
	return err == nil
}

func Cleanup_WorkSpace() bool {
	if _, err := os.Stat(WORKSPACE_NAME); os.IsNotExist(err) {
		return true
	}
	err := os.RemoveAll(WORKSPACE_NAME)
	return err == nil
}

func Initialize_Log_File() error {
	var err error
	log_file, err = os.OpenFile(LOG_NAME, os.O_APPEND|os.O_CREATE|os.O_WRONLY, 0644)
	if err != nil {
		return err
	}
	logHeader := fmt.Sprintf("=== Log Started at %s ===\n", time.Now().Format("2006-01-02 15:04:05"))
	log_file.WriteString(logHeader)
	return nil
}

func Write_Log(message string) error {

	timestamp := time.Now().Format("2006-01-02 15:04:05")
	logEntry := fmt.Sprintf("[%s] %s\n", timestamp, message)
	log_mutex.Lock()
	defer log_mutex.Unlock()
	_, err := log_file.WriteString(logEntry)
	if err != nil {
		return err
	}

	return log_file.Sync()
}

// CloseLogFile 关闭全局日志文件
func Close_Log_File() error {
	if log_file != nil {
		return log_file.Close()
	}
	return nil
}
