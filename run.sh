#!/bin/bash
set -eux

cd ~/wordle-bot

# Run the program and log output
./new-wordle-bot > output.log 2>&1

# Upload logs and results to S3
aws s3 cp output.log s3://dclamage-tempcompute/output.log
aws s3 cp seen_states.bin s3://dclamage-tempcompute/seen_states.bin

# Fetch the instance ID using IMDSv2
TOKEN=$(curl -s -X PUT "http://169.254.169.254/latest/api/token" -H "X-aws-ec2-metadata-token-ttl-seconds: 21600")
INSTANCE_ID=$(curl -s -H "X-aws-ec2-metadata-token: $TOKEN" "http://169.254.169.254/latest/meta-data/instance-id")

# Auto-terminate the instance after completion
aws ec2 terminate-instances --instance-ids $INSTANCE_ID
