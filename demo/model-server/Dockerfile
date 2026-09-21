FROM python:3.13-alpine

ARG MODEL_BEHAVIOR=valid
ENV MODEL_BEHAVIOR=${MODEL_BEHAVIOR}

WORKDIR /app
COPY server.py /app/server.py
COPY invalid-response-v1.json /app/invalid-response-v1.json

EXPOSE 8080
CMD ["python", "/app/server.py"]
