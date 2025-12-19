#Presented by KeJi
#Date : 2025-12-16

import torch
import torch.nn as nn
import os

class Patch_Embedding(nn.Module):
    """
    将图像分割成patches并进行嵌入
    """
    
    def __init__(self, img_size=32, patch_size=4, in_channels=3, embed_dim=192):
        super(Patch_Embedding, self).__init__()
        self.img_size = img_size
        self.patch_size = patch_size
        self.num_patches = (img_size // patch_size) ** 2
        
        # 使用卷积实现patch embedding
        self.projection = nn.Conv2d(in_channels, embed_dim, kernel_size=patch_size, stride=patch_size)
    
    def forward(self, x):
        # x: (batch_size, channels, height, width)
        x = self.projection(x)  # (batch_size, embed_dim, num_patches_h, num_patches_w)
        x = x.flatten(2)  # (batch_size, embed_dim, num_patches)
        x = x.transpose(1, 2)  # (batch_size, num_patches, embed_dim)
        return x


class Multi_Head_Self_Attention(nn.Module):
    """
    多头自注意力机制
    """
    
    def __init__(self, embed_dim=192, num_heads=4):
        super(Multi_Head_Self_Attention, self).__init__()
        self.embed_dim = embed_dim
        self.num_heads = num_heads
        self.head_dim = embed_dim // num_heads
        
        assert embed_dim % num_heads == 0, "embed_dim必须能被num_heads整除"
        
        self.qkv = nn.Linear(embed_dim, embed_dim * 3)
        self.projection = nn.Linear(embed_dim, embed_dim)
    
    def forward(self, x):
        batch_size, num_patches, embed_dim = x.shape
        
        # 生成Q, K, V
        qkv = self.qkv(x)  # (batch_size, num_patches, embed_dim * 3)
        qkv = qkv.reshape(batch_size, num_patches, 3, self.num_heads, self.head_dim)
        qkv = qkv.permute(2, 0, 3, 1, 4)  # (3, batch_size, num_heads, num_patches, head_dim)
        q, k, v = qkv[0], qkv[1], qkv[2]
        
        # 计算注意力分数
        attention = (q @ k.transpose(-2, -1)) / (self.head_dim ** 0.5)
        attention = torch.softmax(attention, dim=-1)
        
        # 应用注意力
        x = attention @ v  # (batch_size, num_heads, num_patches, head_dim)
        x = x.transpose(1, 2)  # (batch_size, num_patches, num_heads, head_dim)
        x = x.reshape(batch_size, num_patches, embed_dim)
        
        # 输出投影
        x = self.projection(x)
        return x


class MLP_Block(nn.Module):
    """
    MLP块
    """
    
    def __init__(self, embed_dim=192, mlp_dim=768, dropout=0.1):
        super(MLP_Block, self).__init__()
        self.fc1 = nn.Linear(embed_dim, mlp_dim)
        self.fc2 = nn.Linear(mlp_dim, embed_dim)
        self.gelu = nn.GELU()
        self.dropout = nn.Dropout(dropout)
    
    def forward(self, x):
        x = self.fc1(x)
        x = self.gelu(x)
        x = self.dropout(x)
        x = self.fc2(x)
        x = self.dropout(x)
        return x


class Transformer_Encoder_Block(nn.Module):
    """
    Transformer编码器块
    """
    
    def __init__(self, embed_dim=192, num_heads=4, mlp_dim=768, dropout=0.1):
        super(Transformer_Encoder_Block, self).__init__()
        self.norm1 = nn.LayerNorm(embed_dim)
        self.attention = Multi_Head_Self_Attention(embed_dim, num_heads)
        self.norm2 = nn.LayerNorm(embed_dim)
        self.mlp = MLP_Block(embed_dim, mlp_dim, dropout)
    
    def forward(self, x):
        # Multi-head self-attention with residual connection
        x = x + self.attention(self.norm1(x))
        # MLP with residual connection
        x = x + self.mlp(self.norm2(x))
        return x


class ViT_Model(nn.Module):
    """
    Vision Transformer模型
    用于图像分类任务
    """
    
    def __init__(self, img_size=32, patch_size=4, in_channels=3, num_classes=10, 
                 embed_dim=192, depth=6, num_heads=4, mlp_dim=768, dropout=0.1):
        super(ViT_Model, self).__init__()
        
        # Patch Embedding
        self.patch_embedding = Patch_Embedding(img_size, patch_size, in_channels, embed_dim)
        num_patches = self.patch_embedding.num_patches
        
        # Class token
        self.cls_token = nn.Parameter(torch.zeros(1, 1, embed_dim))
        
        # Position embedding
        self.position_embedding = nn.Parameter(torch.zeros(1, num_patches + 1, embed_dim))
        
        # Dropout
        self.dropout = nn.Dropout(dropout)
        
        # Transformer Encoder
        self.encoder = nn.ModuleList([
            Transformer_Encoder_Block(embed_dim, num_heads, mlp_dim, dropout)
            for _ in range(depth)
        ])
        
        # Layer Norm
        self.norm = nn.LayerNorm(embed_dim)
        
        # Classification head
        self.head = nn.Linear(embed_dim, num_classes)
    
    def forward(self, x):
        batch_size = x.shape[0]
        
        # Patch embedding
        x = self.patch_embedding(x)  # (batch_size, num_patches, embed_dim)
        
        # Add class token
        cls_tokens = self.cls_token.expand(batch_size, -1, -1)  # (batch_size, 1, embed_dim)
        x = torch.cat([cls_tokens, x], dim=1)  # (batch_size, num_patches + 1, embed_dim)
        
        # Add position embedding
        x = x + self.position_embedding
        x = self.dropout(x)
        
        # Transformer encoder
        for block in self.encoder:
            x = block(x)
        
        # Layer norm
        x = self.norm(x)
        
        # Classification head (use class token)
        x = self.head(x[:, 0])
        
        return x


def Create_And_Save_ViT_Model():
    """
    创建ViT模型，随机初始化参数，并保存到Test/Weights目录
    """
    # 创建模型实例
    model = ViT_Model(
        img_size=32,
        patch_size=4,
        in_channels=3,
        num_classes=10,
        embed_dim=192,
        depth=6,
        num_heads=4,
        mlp_dim=768,
        dropout=0.1
    )
    
    # 模型设置为评估模式
    model.eval()
    
    # 确保Weights目录存在
    weights_dir = os.path.join(os.path.dirname(os.path.dirname(__file__)), 'Weights')
    os.makedirs(weights_dir, exist_ok=True)
    
    # 使用torch.jit.trace保存模型（避免类定义依赖）
    dummy_input = torch.randn(1, 3, 32, 32)
    traced_model = torch.jit.trace(model, dummy_input)
    
    save_path = os.path.join(weights_dir, 'vit_model.pth')
    traced_model.save(save_path)
    
    print(f"ViT模型已保存到: {save_path}")
    print(f"模型参数量: {sum(p.numel() for p in model.parameters()):,}")
    
    return save_path


if __name__ == "__main__":
    Create_And_Save_ViT_Model()
