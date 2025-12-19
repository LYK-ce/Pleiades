#Presented by KeJi
#Date : 2025-12-16

import torch
import torch.nn as nn
import os

class CNN_Model(nn.Module):
    """
    简单的卷积神经网络模型
    用于图像分类任务
    """
    
    def __init__(self, num_classes=10):
        super(CNN_Model, self).__init__()
        
        # 卷积层
        self.conv1 = nn.Conv2d(in_channels=3, out_channels=32, kernel_size=3, padding=1)
        self.conv2 = nn.Conv2d(in_channels=32, out_channels=64, kernel_size=3, padding=1)
        self.conv3 = nn.Conv2d(in_channels=64, out_channels=128, kernel_size=3, padding=1)
        
        # 池化层
        self.pool = nn.MaxPool2d(kernel_size=2, stride=2)
        
        # Batch Normalization
        self.bn1 = nn.BatchNorm2d(32)
        self.bn2 = nn.BatchNorm2d(64)
        self.bn3 = nn.BatchNorm2d(128)
        
        # 全连接层
        self.fc1 = nn.Linear(128 * 4 * 4, 512)
        self.fc2 = nn.Linear(512, num_classes)
        
        # Dropout
        self.dropout = nn.Dropout(0.5)
        
        # 激活函数
        self.relu = nn.ReLU()
    
    def forward(self, x):
        # Conv Block 1
        x = self.conv1(x)
        x = self.bn1(x)
        x = self.relu(x)
        x = self.pool(x)  # 32x32 -> 16x16
        
        # Conv Block 2
        x = self.conv2(x)
        x = self.bn2(x)
        x = self.relu(x)
        x = self.pool(x)  # 16x16 -> 8x8
        
        # Conv Block 3
        x = self.conv3(x)
        x = self.bn3(x)
        x = self.relu(x)
        x = self.pool(x)  # 8x8 -> 4x4
        
        # Flatten
        x = x.view(x.size(0), -1)
        
        # Fully Connected Layers
        x = self.fc1(x)
        x = self.relu(x)
        x = self.dropout(x)
        x = self.fc2(x)
        
        return x


def Create_And_Save_CNN_Model():
    """
    创建CNN模型，随机初始化参数，并保存到Test/Weights目录
    """
    # 创建模型实例
    model = CNN_Model(num_classes=10)
    
    # 模型设置为评估模式
    model.eval()
    
    # 确保Weights目录存在
    weights_dir = os.path.join(os.path.dirname(os.path.dirname(__file__)), 'Weights')
    os.makedirs(weights_dir, exist_ok=True)
    
    # 使用torch.jit.trace保存模型（避免类定义依赖）
    dummy_input = torch.randn(1, 3, 32, 32)
    traced_model = torch.jit.trace(model, dummy_input)
    
    save_path = os.path.join(weights_dir, 'cnn_model.pth')
    traced_model.save(save_path)
    
    print(f"CNN模型已保存到: {save_path}")
    print(f"模型参数量: {sum(p.numel() for p in model.parameters()):,}")
    
    return save_path


if __name__ == "__main__":
    Create_And_Save_CNN_Model()
