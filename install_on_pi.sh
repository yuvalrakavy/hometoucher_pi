#!/bin/bash
sudo cp hometoucher.service /etc/systemd/system
sudo systemctl enable systemd-networkd.service systemd-networkd-wait-online.service hometoucher.service
sudo mv local.network /etc/systemd/network
sudo rm /etc/systemd/system/getty.target.wants/getty*
mkdir logs
sudo shutdown -r now
