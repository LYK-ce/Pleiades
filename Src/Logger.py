#Presented by KeJi
#Date : 2025-12-18

"""
日志组件
提供日志记录功能
"""

import os
from datetime import datetime

class Logger:
    """
    日志记录类
    使用二进制标志位控制日志记录
    """
    
    def __init__(self, log_flag):
        """
        初始化Logger
        
        输入: log_flag - 日志记录标志位（二进制）
        输出: 无
        """
        self.log_flag = log_flag
        
        # 创建Log目录
        log_dir = "Log"
        if not os.path.exists(log_dir):
            os.makedirs(log_dir)
        
        # 创建日志文件，文件名包含当前日期
        current_date = datetime.now().strftime("%Y%m%d")
        log_filename = f"pleiades_{current_date}.log"
        self.log_file = os.path.join(log_dir, log_filename)
        
        # 创建日志文件（如果不存在）
        if not os.path.exists(self.log_file):
            with open(self.log_file, 'w', encoding='utf-8') as f:
                f.write(f"# Pleiades Log File - {current_date}\n")
                f.write(f"# Initialized at {datetime.now().strftime('%Y-%m-%d %H:%M:%S')}\n\n")
    
    def Set_Log_Flag(self, _log_flag):
        """
        设置新的日志记录标志位
        
        输入: _log_flag - 新的日志标志位
        输出: 无
        """
        self.log_flag = _log_flag
    
    def Log(self, flag, _input):
        """
        记录日志
        
        输入: 
            flag - 当前事件的标志位
            _input - 要记录的字符串
        输出: 无
        """
        # 通过按位与操作判断是否需要记录
        if (flag & self.log_flag) != 0:
            # 获取当前时间
            current_time = datetime.now().strftime("%Y-%m-%d %H:%M:%S.%f")[:-3]
            
            # 格式化日志内容：date:_input
            log_entry = f"{current_time}: {_input}\n"
            
            # 写入日志文件
            try:
                with open(self.log_file, 'a', encoding='utf-8') as f:
                    f.write(log_entry)
            except Exception as e:
                print(f"日志写入失败: {e}")


# 全局logger变量，初始值为None
logger = None
