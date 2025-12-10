import torch
import torch.nn as nn
import torch.optim as optim
import pandas as pd
import numpy as np
from pathlib import Path
import os

# Define the model (must match input dim of 3)
class FraudModel(nn.Module):
    def __init__(self):
        super(FraudModel, self).__init__()
        self.layer1 = nn.Linear(3, 16)
        self.relu = nn.ReLU()
        self.layer2 = nn.Linear(16, 1)
        self.sigmoid = nn.Sigmoid()

    def forward(self, x):
        x = self.layer1(x)
        x = self.relu(x)
        x = self.layer2(x)
        x = self.sigmoid(x)
        return x

def train_and_export():
    # Paths
    data_path = Path("data/training/latest/training_sample.parquet")
    output_dir = Path("models/fraud/latest")
    output_dir.mkdir(parents=True, exist_ok=True)
    onnx_path = output_dir / "fraud_model.onnx"

    print(f"Loading data from {data_path}...")
    df = pd.read_parquet(data_path)

    # Feature selection and engineering
    # Mapping to feature_1, feature_2, feature_3 as expected by inference service
    # feature_1: Network entropy -> using auction_bid_count as proxy
    # feature_2: CTIT -> using ctit_seconds
    # feature_3: IP score -> using Tor exit node match (1.0 if Tor, 0.0 otherwise)
    
    # Load Tor exit nodes
    tor_dir = Path("data/enrichment/v1/tor")
    # Find latest date directory
    if tor_dir.exists():
        latest_date_dir = sorted([d for d in tor_dir.iterdir() if d.is_dir()])[-1]
        tor_list_path = latest_date_dir / "torbulkexitlist.txt"
        print(f"Loading Tor exit nodes from {tor_list_path}...")
        with open(tor_list_path, "r") as f:
            tor_ips = set(line.strip() for line in f if line.strip())
    else:
        print("Warning: Tor enrichment data not found. Using empty set.")
        tor_ips = set()

    df['is_tor'] = df['ip'].apply(lambda x: 1.0 if x in tor_ips else 0.0)

    features = df[['auction_bid_count', 'ctit_seconds', 'is_tor']].copy()
    features = features.fillna(0)
    
    X = torch.tensor(features.values, dtype=torch.float32)
    y = torch.tensor(df['label'].values, dtype=torch.float32).unsqueeze(1)

    # Train
    model = FraudModel()
    criterion = nn.BCELoss()
    optimizer = optim.Adam(model.parameters(), lr=0.01)

    print("Training model...")
    for epoch in range(100):
        optimizer.zero_grad()
        outputs = model(X)
        loss = criterion(outputs, y)
        loss.backward()
        optimizer.step()
        
        if epoch % 10 == 0:
            print(f"Epoch {epoch}, Loss: {loss.item()}")

    # Export to ONNX
    print(f"Exporting to {onnx_path}...")
    dummy_input = torch.randn(1, 3)
    torch.onnx.export(
        model, 
        dummy_input, 
        str(onnx_path),
        input_names=['input'],
        output_names=['output'],
        dynamic_axes={'input': {0: 'batch_size'}, 'output': {0: 'batch_size'}}
    )
    print("Done.")

if __name__ == "__main__":
    train_and_export()
